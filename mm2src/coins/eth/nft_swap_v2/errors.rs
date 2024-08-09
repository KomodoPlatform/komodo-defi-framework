pub(crate) use crate::eth::eth_swap_v2::PrepareTxDataError;

#[derive(Debug, Display)]
pub(crate) enum Erc721FunctionError {
    AbiError(String),
    FunctionNotFound(String),
}

#[derive(Debug, Display)]
pub(crate) enum HtlcParamsError {
    WrongPaymentTx(String),
    TxDeserializationError(String),
}

impl From<Erc721FunctionError> for PrepareTxDataError {
    fn from(e: Erc721FunctionError) -> Self {
        match e {
            Erc721FunctionError::AbiError(e) => PrepareTxDataError::AbiError(e),
            Erc721FunctionError::FunctionNotFound(e) => PrepareTxDataError::Internal(e),
        }
    }
}
