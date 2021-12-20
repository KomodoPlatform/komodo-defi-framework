use common::mm_error::prelude::*;

pub type LedgerResult<T> = Result<T, MmError<LedgerError>>;

#[derive(Debug, Display)]
pub enum LedgerError {
    /// TODO put a device info
    DeviceDisconnected,
    /// The error depends on transport implementation.
    UnderlyingError(String),
    ErrorDeserializingApdu(String),
    ProtocolError(String),
    InternalError(String),
}
