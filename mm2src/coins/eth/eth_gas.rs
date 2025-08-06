use super::*;

/// Take into account that the dynamic fee may increase by 3% during the swap.
pub(super) const GAS_PRICE_APPROXIMATION_PERCENT_ON_START_SWAP: u64 = 3;
/// Take into account that the dynamic fee may increase until the locktime is expired
pub(super) const GAS_PRICE_APPROXIMATION_PERCENT_ON_WATCHER_PREIMAGE: u64 = 3;
/// Take into account that the dynamic fee may increase at each of the following stages:
/// - it may increase by 2% until a swap is started;
/// - it may increase by 3% during the swap.
pub(super) const GAS_PRICE_APPROXIMATION_PERCENT_ON_ORDER_ISSUE: u64 = 5;
/// Take into account that the dynamic fee may increase at each of the following stages:
/// - it may increase by 2% until an order is issued;
/// - it may increase by 2% until a swap is started;
/// - it may increase by 3% during the swap.
pub(super) const GAS_PRICE_APPROXIMATION_PERCENT_ON_TRADE_PREIMAGE: u64 = 7;

/// Heuristic default gas limits for withdraw and swap operations (including extra margin value for possible changes in opcodes cost)
pub mod gas_limit {
    /// Gas limit for sending coins
    pub const ETH_SEND_COINS: u64 = 21_000;
    /// Gas limit for transfer ERC20 tokens
    /// TODO: maybe this is too much and 150K is okay
    pub const ETH_SEND_ERC20: u64 = 210_000;
    /// Gas limit for swap payment tx with coins
    /// real values are approx 48,6K by etherscan
    pub const ETH_PAYMENT: u64 = 65_000;
    /// Gas limit for swap payment tx with ERC20 tokens
    /// real values are 98,9K for ERC20 and 135K for ERC-1967 proxied ERC20 contracts (use 'gas_limit' override in coins to tune)
    pub const ERC20_PAYMENT: u64 = 150_000;
    /// Gas limit for swap receiver spend tx with coins
    /// real values are 40,7K
    pub const ETH_RECEIVER_SPEND: u64 = 65_000;
    /// Gas limit for swap receiver spend tx with ERC20 tokens
    /// real values are 72,8K
    pub const ERC20_RECEIVER_SPEND: u64 = 150_000;
    /// Gas limit for swap refund tx with coins
    pub const ETH_SENDER_REFUND: u64 = 100_000;
    /// Gas limit for swap refund tx with ERC20 tokens
    pub const ERC20_SENDER_REFUND: u64 = 150_000;
    /// Gas limit for other operations
    pub const ETH_MAX_TRADE_GAS: u64 = 150_000;
}

/// Default gas limits for EthGasLimitV2
pub mod gas_limit_v2 {
    /// Gas limits for maker operations in EtomicSwapMakerV2 contract
    pub mod maker {
        pub const ETH_PAYMENT: u64 = 65_000;
        pub const ERC20_PAYMENT: u64 = 150_000;
        pub const ETH_TAKER_SPEND: u64 = 100_000;
        pub const ERC20_TAKER_SPEND: u64 = 150_000;
        pub const ETH_MAKER_REFUND_TIMELOCK: u64 = 90_000;
        pub const ERC20_MAKER_REFUND_TIMELOCK: u64 = 100_000;
        pub const ETH_MAKER_REFUND_SECRET: u64 = 90_000;
        pub const ERC20_MAKER_REFUND_SECRET: u64 = 100_000;
    }

    /// Gas limits for taker operations in EtomicSwapTakerV2 contract
    pub mod taker {
        pub const ETH_PAYMENT: u64 = 65_000;
        pub const ERC20_PAYMENT: u64 = 150_000;
        pub const ETH_MAKER_SPEND: u64 = 100_000;
        pub const ERC20_MAKER_SPEND: u64 = 115_000;
        pub const ETH_TAKER_REFUND_TIMELOCK: u64 = 90_000;
        pub const ERC20_TAKER_REFUND_TIMELOCK: u64 = 100_000;
        pub const ETH_TAKER_REFUND_SECRET: u64 = 90_000;
        pub const ERC20_TAKER_REFUND_SECRET: u64 = 100_000;
        pub const APPROVE_PAYMENT: u64 = 50_000;
    }

    pub mod nft_maker {
        pub const ERC721_PAYMENT: u64 = 130_000;
        pub const ERC1155_PAYMENT: u64 = 130_000;
        pub const ERC721_TAKER_SPEND: u64 = 100_000;
        pub const ERC1155_TAKER_SPEND: u64 = 100_000;
        pub const ERC721_MAKER_REFUND_TIMELOCK: u64 = 100_000;
        pub const ERC1155_MAKER_REFUND_TIMELOCK: u64 = 100_000;
        pub const ERC721_MAKER_REFUND_SECRET: u64 = 100_000;
        pub const ERC1155_MAKER_REFUND_SECRET: u64 = 100_000;
    }
}

/// Coin conf param to override default gas limits
#[derive(Deserialize)]
#[serde(default)]
pub(crate) struct EthGasLimit {
    /// Gas limit for sending coins
    pub eth_send_coins: u64,
    /// Gas limit for sending ERC20 tokens
    pub eth_send_erc20: u64,
    /// Gas limit for swap payment tx with coins
    pub eth_payment: u64,
    /// Gas limit for swap payment tx with ERC20 tokens
    pub erc20_payment: u64,
    /// Gas limit for swap receiver spend tx with coins
    pub eth_receiver_spend: u64,
    /// Gas limit for swap receiver spend tx with ERC20 tokens
    pub erc20_receiver_spend: u64,
    /// Gas limit for swap refund tx with coins
    pub eth_sender_refund: u64,
    /// Gas limit for swap refund tx with ERC20 tokens
    pub erc20_sender_refund: u64,
    /// Gas limit for other operations
    pub eth_max_trade_gas: u64,
}

impl Default for EthGasLimit {
    fn default() -> Self {
        EthGasLimit {
            eth_send_coins: gas_limit::ETH_SEND_COINS,
            eth_send_erc20: gas_limit::ETH_SEND_ERC20,
            eth_payment: gas_limit::ETH_PAYMENT,
            erc20_payment: gas_limit::ERC20_PAYMENT,
            eth_receiver_spend: gas_limit::ETH_RECEIVER_SPEND,
            erc20_receiver_spend: gas_limit::ERC20_RECEIVER_SPEND,
            eth_sender_refund: gas_limit::ETH_SENDER_REFUND,
            erc20_sender_refund: gas_limit::ERC20_SENDER_REFUND,
            eth_max_trade_gas: gas_limit::ETH_MAX_TRADE_GAS,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(crate) struct EthGasLimitV2 {
    pub maker: MakerGasLimitV2,
    pub taker: TakerGasLimitV2,
    pub nft_maker: NftMakerGasLimitV2,
}

#[derive(Deserialize)]
#[serde(default)]
pub(crate) struct MakerGasLimitV2 {
    pub eth_payment: u64,
    pub erc20_payment: u64,
    pub eth_taker_spend: u64,
    pub erc20_taker_spend: u64,
    pub eth_maker_refund_timelock: u64,
    pub erc20_maker_refund_timelock: u64,
    pub eth_maker_refund_secret: u64,
    pub erc20_maker_refund_secret: u64,
}

#[derive(Deserialize)]
#[serde(default)]
pub(crate) struct TakerGasLimitV2 {
    pub eth_payment: u64,
    pub erc20_payment: u64,
    pub eth_maker_spend: u64,
    pub erc20_maker_spend: u64,
    pub eth_taker_refund_timelock: u64,
    pub erc20_taker_refund_timelock: u64,
    pub eth_taker_refund_secret: u64,
    pub erc20_taker_refund_secret: u64,
    pub approve_payment: u64,
}

#[derive(Deserialize)]
#[serde(default)]
pub(crate) struct NftMakerGasLimitV2 {
    pub erc721_payment: u64,
    pub erc1155_payment: u64,
    pub erc721_taker_spend: u64,
    pub erc1155_taker_spend: u64,
    pub erc721_maker_refund_timelock: u64,
    pub erc1155_maker_refund_timelock: u64,
    pub erc721_maker_refund_secret: u64,
    pub erc1155_maker_refund_secret: u64,
}

impl EthGasLimitV2 {
    pub(crate) fn gas_limit(
        &self,
        coin_type: &EthCoinType,
        payment_type: EthPaymentType,
        method: PaymentMethod,
    ) -> Result<u64, String> {
        match coin_type {
            EthCoinType::Eth => {
                let gas_limit = match payment_type {
                    EthPaymentType::MakerPayments => match method {
                        PaymentMethod::Send => self.maker.eth_payment,
                        PaymentMethod::Spend => self.maker.eth_taker_spend,
                        PaymentMethod::RefundTimelock => self.maker.eth_maker_refund_timelock,
                        PaymentMethod::RefundSecret => self.maker.eth_maker_refund_secret,
                    },
                    EthPaymentType::TakerPayments => match method {
                        PaymentMethod::Send => self.taker.eth_payment,
                        PaymentMethod::Spend => self.taker.eth_maker_spend,
                        PaymentMethod::RefundTimelock => self.taker.eth_taker_refund_timelock,
                        PaymentMethod::RefundSecret => self.taker.eth_taker_refund_secret,
                    },
                };
                Ok(gas_limit)
            },
            EthCoinType::Erc20 { .. } => {
                let gas_limit = match payment_type {
                    EthPaymentType::MakerPayments => match method {
                        PaymentMethod::Send => self.maker.erc20_payment,
                        PaymentMethod::Spend => self.maker.erc20_taker_spend,
                        PaymentMethod::RefundTimelock => self.maker.erc20_maker_refund_timelock,
                        PaymentMethod::RefundSecret => self.maker.erc20_maker_refund_secret,
                    },
                    EthPaymentType::TakerPayments => match method {
                        PaymentMethod::Send => self.taker.erc20_payment,
                        PaymentMethod::Spend => self.taker.erc20_maker_spend,
                        PaymentMethod::RefundTimelock => self.taker.erc20_taker_refund_timelock,
                        PaymentMethod::RefundSecret => self.taker.erc20_taker_refund_secret,
                    },
                };
                Ok(gas_limit)
            },
            EthCoinType::Nft { .. } => Err("NFT protocol is not supported for ETH and ERC20 Swaps".to_string()),
        }
    }

    pub(crate) fn nft_gas_limit(&self, contract_type: &ContractType, method: PaymentMethod) -> u64 {
        match contract_type {
            ContractType::Erc1155 => match method {
                PaymentMethod::Send => self.nft_maker.erc1155_payment,
                PaymentMethod::Spend => self.nft_maker.erc1155_taker_spend,
                PaymentMethod::RefundTimelock => self.nft_maker.erc1155_maker_refund_timelock,
                PaymentMethod::RefundSecret => self.nft_maker.erc1155_maker_refund_secret,
            },
            ContractType::Erc721 => match method {
                PaymentMethod::Send => self.nft_maker.erc721_payment,
                PaymentMethod::Spend => self.nft_maker.erc721_taker_spend,
                PaymentMethod::RefundTimelock => self.nft_maker.erc721_maker_refund_timelock,
                PaymentMethod::RefundSecret => self.nft_maker.erc721_maker_refund_secret,
            },
        }
    }
}

impl Default for MakerGasLimitV2 {
    fn default() -> Self {
        MakerGasLimitV2 {
            eth_payment: gas_limit_v2::maker::ETH_PAYMENT,
            erc20_payment: gas_limit_v2::maker::ERC20_PAYMENT,
            eth_taker_spend: gas_limit_v2::maker::ETH_TAKER_SPEND,
            erc20_taker_spend: gas_limit_v2::maker::ERC20_TAKER_SPEND,
            eth_maker_refund_timelock: gas_limit_v2::maker::ETH_MAKER_REFUND_TIMELOCK,
            erc20_maker_refund_timelock: gas_limit_v2::maker::ERC20_MAKER_REFUND_TIMELOCK,
            eth_maker_refund_secret: gas_limit_v2::maker::ETH_MAKER_REFUND_SECRET,
            erc20_maker_refund_secret: gas_limit_v2::maker::ERC20_MAKER_REFUND_SECRET,
        }
    }
}

impl Default for TakerGasLimitV2 {
    fn default() -> Self {
        TakerGasLimitV2 {
            eth_payment: gas_limit_v2::taker::ETH_PAYMENT,
            erc20_payment: gas_limit_v2::taker::ERC20_PAYMENT,
            eth_maker_spend: gas_limit_v2::taker::ETH_MAKER_SPEND,
            erc20_maker_spend: gas_limit_v2::taker::ERC20_MAKER_SPEND,
            eth_taker_refund_timelock: gas_limit_v2::taker::ETH_TAKER_REFUND_TIMELOCK,
            erc20_taker_refund_timelock: gas_limit_v2::taker::ERC20_TAKER_REFUND_TIMELOCK,
            eth_taker_refund_secret: gas_limit_v2::taker::ETH_TAKER_REFUND_SECRET,
            erc20_taker_refund_secret: gas_limit_v2::taker::ERC20_TAKER_REFUND_SECRET,
            approve_payment: gas_limit_v2::taker::APPROVE_PAYMENT,
        }
    }
}

impl Default for NftMakerGasLimitV2 {
    fn default() -> Self {
        NftMakerGasLimitV2 {
            erc721_payment: gas_limit_v2::nft_maker::ERC721_PAYMENT,
            erc1155_payment: gas_limit_v2::nft_maker::ERC1155_PAYMENT,
            erc721_taker_spend: gas_limit_v2::nft_maker::ERC721_TAKER_SPEND,
            erc1155_taker_spend: gas_limit_v2::nft_maker::ERC1155_TAKER_SPEND,
            erc721_maker_refund_timelock: gas_limit_v2::nft_maker::ERC721_MAKER_REFUND_TIMELOCK,
            erc1155_maker_refund_timelock: gas_limit_v2::nft_maker::ERC1155_MAKER_REFUND_TIMELOCK,
            erc721_maker_refund_secret: gas_limit_v2::nft_maker::ERC721_MAKER_REFUND_SECRET,
            erc1155_maker_refund_secret: gas_limit_v2::nft_maker::ERC1155_MAKER_REFUND_SECRET,
        }
    }
}

pub(crate) trait ExtractGasLimit: Default + for<'de> Deserialize<'de> {
    fn key() -> &'static str;
}

impl ExtractGasLimit for EthGasLimit {
    fn key() -> &'static str {
        "gas_limit"
    }
}

impl ExtractGasLimit for EthGasLimitV2 {
    fn key() -> &'static str {
        "gas_limit_v2"
    }
}

/// Gas price multipliers to adjust gas price estimation per coin basis
#[derive(Clone, Debug, Deserialize)]
pub(super) struct GasPriceAdjust {
    /// Multiplier for legacy gas price
    pub(super) legacy_price_mult: f64,
    /// Multipliers for 3 levels of base fee
    pub(super) base_fee_mult: [f64; FEE_PRIORITY_LEVEL_N],
    /// Multipliers for 3 levels of max priority fee
    pub(super) priority_fee_mult: [f64; FEE_PRIORITY_LEVEL_N],
}

/// Internal structure describing how transaction pays for gas unit:
/// either legacy gas price or EIP-1559 fee per gas
#[derive(Clone, Debug)]
pub(crate) enum PayForGasOption {
    /// Legacy transaction gas price
    Legacy { gas_price: U256 },
    /// Fee per gas option introduced in https://eips.ethereum.org/EIPS/eip-1559
    Eip1559 {
        max_fee_per_gas: U256,
        max_priority_fee_per_gas: U256,
    },
}

impl PayForGasOption {
    pub(crate) fn get_gas_price(&self) -> Option<U256> {
        match self {
            PayForGasOption::Legacy { gas_price } => Some(*gas_price),
            PayForGasOption::Eip1559 { .. } => None,
        }
    }

    pub(crate) fn get_fee_per_gas(&self) -> (Option<U256>, Option<U256>) {
        match self {
            PayForGasOption::Eip1559 {
                max_fee_per_gas,
                max_priority_fee_per_gas,
            } => (Some(*max_fee_per_gas), Some(*max_priority_fee_per_gas)),
            PayForGasOption::Legacy { .. } => (None, None),
        }
    }
}

pub(super) type GasDetails = (U256, PayForGasOption);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EthTxFeeDetails {
    pub coin: String,
    pub gas: u64,
    /// Gas price in ETH per gas unit
    /// if 'max_fee_per_gas' and 'max_priority_fee_per_gas' are used we set 'gas_price' as 'max_fee_per_gas' for compatibility with GUI
    pub gas_price: BigDecimal,
    /// Max fee per gas in ETH per gas unit
    pub max_fee_per_gas: Option<BigDecimal>,
    /// Max priority fee per gas in ETH per gas unit
    pub max_priority_fee_per_gas: Option<BigDecimal>,
    pub total_fee: BigDecimal,
}

impl EthTxFeeDetails {
    pub(crate) fn new(gas: U256, pay_for_gas_option: PayForGasOption, coin: &str) -> NumConversResult<EthTxFeeDetails> {
        let total_fee = calc_total_fee(gas, &pay_for_gas_option)?;
        // Fees are always paid in ETH, can use 18 decimals by default
        let total_fee = u256_to_big_decimal(total_fee, ETH_DECIMALS)?;
        let (gas_price, max_fee_per_gas, max_priority_fee_per_gas) = match pay_for_gas_option {
            PayForGasOption::Legacy { gas_price } => (gas_price, None, None),
            // Using max_fee_per_gas as estimated gas_price value for compatibility in caller not expecting eip1559 fee per gas values.
            // Normally the caller should pay attention to presence of max_fee_per_gas and max_priority_fee_per_gas in the result:
            PayForGasOption::Eip1559 {
                max_fee_per_gas,
                max_priority_fee_per_gas,
            } => (max_fee_per_gas, Some(max_fee_per_gas), Some(max_priority_fee_per_gas)),
        };
        let gas_price = u256_to_big_decimal(gas_price, ETH_DECIMALS)?;
        let (max_fee_per_gas, max_priority_fee_per_gas) = match (max_fee_per_gas, max_priority_fee_per_gas) {
            (Some(max_fee_per_gas), Some(max_priority_fee_per_gas)) => (
                Some(u256_to_big_decimal(max_fee_per_gas, ETH_DECIMALS)?),
                Some(u256_to_big_decimal(max_priority_fee_per_gas, ETH_DECIMALS)?),
            ),
            (_, _) => (None, None),
        };
        let gas_u64 = u64::try_from(gas).map_to_mm(|e| NumConversError::new(e.to_string()))?;

        Ok(EthTxFeeDetails {
            coin: coin.to_owned(),
            gas: gas_u64,
            gas_price,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            total_fee,
        })
    }
}
