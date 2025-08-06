use super::*;

#[derive(Debug, Display, EnumFromStringify, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum Web3RpcError {
    #[display(fmt = "Transport: {_0}")]
    Transport(String),
    #[display(fmt = "Invalid response: {_0}")]
    InvalidResponse(String),
    #[display(fmt = "Timeout: {_0}")]
    Timeout(String),
    #[from_stringify("serde_json::Error")]
    #[display(fmt = "Internal: {_0}")]
    Internal(String),
    #[display(fmt = "Invalid gas api provider config: {_0}")]
    InvalidGasApiConfig(String),
    #[display(fmt = "Nft Protocol is not supported yet!")]
    NftProtocolNotSupported,
    #[display(fmt = "Number conversion: {_0}")]
    NumConversError(String),
    #[display(fmt = "No such coin {coin}")]
    NoSuchCoin { coin: String },
}

impl From<web3::Error> for Web3RpcError {
    fn from(e: web3::Error) -> Self {
        let error_str = e.to_string();
        match e {
            web3::Error::InvalidResponse(_) | web3::Error::Decoder(_) | web3::Error::Rpc(_) => {
                Web3RpcError::InvalidResponse(error_str)
            },
            web3::Error::Unreachable | web3::Error::Transport(_) | web3::Error::Io(_) => {
                Web3RpcError::Transport(error_str)
            },
            _ => Web3RpcError::Internal(error_str),
        }
    }
}

impl From<Web3RpcError> for RawTransactionError {
    fn from(e: Web3RpcError) -> Self {
        match e {
            Web3RpcError::Transport(tr) | Web3RpcError::InvalidResponse(tr) => RawTransactionError::Transport(tr),
            Web3RpcError::Internal(internal)
            | Web3RpcError::Timeout(internal)
            | Web3RpcError::NumConversError(internal)
            | Web3RpcError::InvalidGasApiConfig(internal) => RawTransactionError::InternalError(internal),
            Web3RpcError::NoSuchCoin { coin } => RawTransactionError::NoSuchCoin { coin },
            Web3RpcError::NftProtocolNotSupported => {
                RawTransactionError::InternalError("Nft Protocol is not supported yet!".to_string())
            },
        }
    }
}

impl From<ethabi::Error> for Web3RpcError {
    fn from(e: ethabi::Error) -> Web3RpcError {
        // Currently, we use the `ethabi` crate to work with a smart contract ABI known at compile time.
        // It's an internal error if there are any issues during working with a smart contract ABI.
        Web3RpcError::Internal(e.to_string())
    }
}

impl From<UnexpectedDerivationMethod> for Web3RpcError {
    fn from(e: UnexpectedDerivationMethod) -> Self {
        Web3RpcError::Internal(e.to_string())
    }
}

#[cfg(target_arch = "wasm32")]
impl From<MetamaskError> for Web3RpcError {
    fn from(e: MetamaskError) -> Self {
        match e {
            MetamaskError::Internal(internal) => Web3RpcError::Internal(internal),
            other => Web3RpcError::Transport(other.to_string()),
        }
    }
}

impl From<NumConversError> for Web3RpcError {
    fn from(e: NumConversError) -> Self {
        Web3RpcError::NumConversError(e.to_string())
    }
}

impl From<CoinFindError> for Web3RpcError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => Web3RpcError::NoSuchCoin { coin },
        }
    }
}

impl From<ethabi::Error> for WithdrawError {
    fn from(e: ethabi::Error) -> Self {
        // Currently, we use the `ethabi` crate to work with a smart contract ABI known at compile time.
        // It's an internal error if there are any issues during working with a smart contract ABI.
        WithdrawError::InternalError(e.to_string())
    }
}

impl From<web3::Error> for WithdrawError {
    fn from(e: web3::Error) -> Self {
        WithdrawError::Transport(e.to_string())
    }
}

impl From<Web3RpcError> for WithdrawError {
    fn from(e: Web3RpcError) -> Self {
        match e {
            Web3RpcError::Transport(err) | Web3RpcError::InvalidResponse(err) => WithdrawError::Transport(err),
            Web3RpcError::Internal(internal)
            | Web3RpcError::Timeout(internal)
            | Web3RpcError::NumConversError(internal)
            | Web3RpcError::InvalidGasApiConfig(internal) => WithdrawError::InternalError(internal),
            Web3RpcError::NoSuchCoin { coin } => WithdrawError::NoSuchCoin { coin },
            Web3RpcError::NftProtocolNotSupported => WithdrawError::NftProtocolNotSupported,
        }
    }
}

impl From<ethcore_transaction::Error> for WithdrawError {
    fn from(e: ethcore_transaction::Error) -> Self {
        WithdrawError::SigningError(e.to_string())
    }
}

impl From<web3::Error> for TradePreimageError {
    fn from(e: web3::Error) -> Self {
        TradePreimageError::Transport(e.to_string())
    }
}

impl From<Web3RpcError> for TradePreimageError {
    fn from(e: Web3RpcError) -> Self {
        match e {
            Web3RpcError::Transport(err) | Web3RpcError::InvalidResponse(err) => TradePreimageError::Transport(err),
            Web3RpcError::Internal(internal)
            | Web3RpcError::Timeout(internal)
            | Web3RpcError::NumConversError(internal)
            | Web3RpcError::InvalidGasApiConfig(internal) => TradePreimageError::InternalError(internal),
            Web3RpcError::NoSuchCoin { coin } => TradePreimageError::NoSuchCoin { coin },
            Web3RpcError::NftProtocolNotSupported => TradePreimageError::NftProtocolNotSupported,
        }
    }
}

impl From<ethabi::Error> for TradePreimageError {
    fn from(e: ethabi::Error) -> Self {
        // Currently, we use the `ethabi` crate to work with a smart contract ABI known at compile time.
        // It's an internal error if there are any issues during working with a smart contract ABI.
        TradePreimageError::InternalError(e.to_string())
    }
}

impl From<ethabi::Error> for BalanceError {
    fn from(e: ethabi::Error) -> Self {
        // Currently, we use the `ethabi` crate to work with a smart contract ABI known at compile time.
        // It's an internal error if there are any issues during working with a smart contract ABI.
        BalanceError::Internal(e.to_string())
    }
}

impl From<web3::Error> for BalanceError {
    fn from(e: web3::Error) -> Self {
        BalanceError::from(Web3RpcError::from(e))
    }
}

impl From<Web3RpcError> for BalanceError {
    fn from(e: Web3RpcError) -> Self {
        match e {
            Web3RpcError::Transport(tr) | Web3RpcError::InvalidResponse(tr) => BalanceError::Transport(tr),
            Web3RpcError::Internal(internal)
            | Web3RpcError::Timeout(internal)
            | Web3RpcError::NumConversError(internal)
            | Web3RpcError::InvalidGasApiConfig(internal) => BalanceError::Internal(internal),
            Web3RpcError::NoSuchCoin { coin } => BalanceError::NoSuchCoin { coin },
            Web3RpcError::NftProtocolNotSupported => {
                BalanceError::Internal("Nft Protocol is not supported yet!".to_string())
            },
        }
    }
}

impl From<TxBuilderError> for TransactionErr {
    fn from(e: TxBuilderError) -> Self {
        TransactionErr::Plain(e.to_string())
    }
}

impl From<ethcore_transaction::Error> for TransactionErr {
    fn from(e: ethcore_transaction::Error) -> Self {
        TransactionErr::Plain(e.to_string())
    }
}

impl From<crate::CoinFindError> for TransactionErr {
    fn from(e: crate::CoinFindError) -> Self {
        TransactionErr::Plain(e.to_string())
    }
}

/// Represents errors that can occur while retrieving an Ethereum address.
#[derive(Clone, Debug, Deserialize, Display, PartialEq, Serialize)]
pub enum GetEthAddressError {
    UnexpectedDerivationMethod(UnexpectedDerivationMethod),
    EthActivationV2Error(EthActivationV2Error),
    Internal(String),
}

impl From<UnexpectedDerivationMethod> for GetEthAddressError {
    fn from(e: UnexpectedDerivationMethod) -> Self {
        GetEthAddressError::UnexpectedDerivationMethod(e)
    }
}

impl From<EthActivationV2Error> for GetEthAddressError {
    fn from(e: EthActivationV2Error) -> Self {
        GetEthAddressError::EthActivationV2Error(e)
    }
}

impl From<CryptoCtxError> for GetEthAddressError {
    fn from(e: CryptoCtxError) -> Self {
        GetEthAddressError::Internal(e.to_string())
    }
}

#[derive(Clone, Debug, Deserialize, Display, EnumFromStringify, PartialEq, Serialize)]
pub enum EthGasDetailsErr {
    #[display(fmt = "Invalid fee policy: {_0}")]
    InvalidFeePolicy(String),
    #[display(fmt = "Amount {amount} is too low. Required minimum is {threshold} to cover fees.")]
    AmountTooLow { amount: BigDecimal, threshold: BigDecimal },
    #[display(
        fmt = "Provided gas fee cap {provided_fee_cap} Gwei is too low, the required network base fee is {required_base_fee} Gwei."
    )]
    GasFeeCapTooLow {
        provided_fee_cap: BigDecimal,
        required_base_fee: BigDecimal,
    },
    #[display(fmt = "The provided 'max_fee_per_gas' is below the current block's base fee.")]
    GasFeeCapBelowBaseFee,
    #[from_stringify("NumConversError")]
    #[display(fmt = "Internal error: {_0}")]
    Internal(String),
    #[display(fmt = "Transport: {_0}")]
    Transport(String),
    #[display(fmt = "Nft Protocol is not supported yet!")]
    NftProtocolNotSupported,
    #[display(fmt = "No such coin {coin}")]
    NoSuchCoin { coin: String },
}

impl From<web3::Error> for EthGasDetailsErr {
    fn from(e: web3::Error) -> Self {
        EthGasDetailsErr::from(Web3RpcError::from(e))
    }
}

impl From<Web3RpcError> for EthGasDetailsErr {
    fn from(e: Web3RpcError) -> Self {
        match e {
            Web3RpcError::Transport(tr) | Web3RpcError::InvalidResponse(tr) => EthGasDetailsErr::Transport(tr),
            Web3RpcError::Internal(internal)
            | Web3RpcError::Timeout(internal)
            | Web3RpcError::NumConversError(internal)
            | Web3RpcError::InvalidGasApiConfig(internal) => EthGasDetailsErr::Internal(internal),
            Web3RpcError::NoSuchCoin { coin } => EthGasDetailsErr::NoSuchCoin { coin },
            Web3RpcError::NftProtocolNotSupported => EthGasDetailsErr::NftProtocolNotSupported,
        }
    }
}

#[derive(Debug, Display, EnumFromStringify)]
pub enum EthAssocTypesError {
    InvalidHexString(String),
    #[from_stringify("DecoderError")]
    TxParseError(String),
    ParseSignatureError(String),
}

#[derive(Debug, Display)]
pub enum EthNftAssocTypesError {
    Utf8Error(String),
    ParseContractTypeError(ParseContractTypeError),
    ParseTokenContractError(String),
}

impl From<ParseContractTypeError> for EthNftAssocTypesError {
    fn from(e: ParseContractTypeError) -> Self {
        EthNftAssocTypesError::ParseContractTypeError(e)
    }
}

/// Errors encountered while validating Ethereum addresses for NFT withdrawal.
#[derive(Display)]
pub enum GetValidEthWithdrawAddError {
    /// The specified coin does not support NFT withdrawal.
    #[display(fmt = "{coin} coin doesn't support NFT withdrawing")]
    CoinDoesntSupportNftWithdraw { coin: String },
    /// The provided address is invalid.
    InvalidAddress(String),
}
