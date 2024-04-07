use enum_derives::EnumFromStringify;

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

#[derive(Debug, Display, EnumFromStringify)]
pub(crate) enum PaymentStatusErr {
    #[from_stringify("ethabi::Error")]
    #[display(fmt = "Abi error: {}", _0)]
    AbiError(String),
    #[from_stringify("web3::Error")]
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
    #[display(fmt = "Tx deserialization error: {}", _0)]
    TxDeserializationError(String),
}

#[derive(Debug, Display, EnumFromStringify)]
pub(crate) enum PrepareTxDataError {
    #[from_stringify("ethabi::Error")]
    #[display(fmt = "Abi error: {}", _0)]
    AbiError(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
    Erc721FunctionError(Erc721FunctionError),
}

impl From<Erc721FunctionError> for PrepareTxDataError {
    fn from(e: Erc721FunctionError) -> Self { Self::Erc721FunctionError(e) }
}
