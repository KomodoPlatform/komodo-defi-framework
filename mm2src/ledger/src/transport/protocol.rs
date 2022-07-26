use crate::error::LedgerResult;
use crate::transport::apdu::{APDUAnswer, APDUCommand};
use byteorder::{BigEndian, ByteOrder};

const CHUNK_SIZE: usize = 64;
const CHUNK_LEN: u32 = 64;

/// A link represents a serial connection to send and receive byte chunks from and to a Ledger device.
#[async_trait]
pub trait Link {
    async fn write_chunk(&mut self, chunk: Vec<u8>) -> LedgerResult<()>;
    async fn read_chunk(&mut self, chunk_len: u32) -> LedgerResult<Vec<u8>>;
}

/// A protocol is used to encode messages in chunks that can be sent to the device and to parse
/// chunks into messages.
#[async_trait]
pub trait Protocol {
    async fn write(&mut self, message: APDUCommand) -> LedgerResult<()>;
    async fn read(&mut self) -> LedgerResult<APDUAnswer>;
}

pub struct HidProtocol<L: Link> {
    pub link: L,
}

impl<L: Link> Protocol for HidProtocol<L> {
    async fn write(&mut self, message: APDUCommand) -> LedgerResult<()> { todo!() }

    async fn read(&mut self) -> LedgerResult<APDUAnswer> { todo!() }
}
