use crate::{LedgerError, LedgerResult};
use byteorder::{BigEndian, ByteOrder};
use mm2_ehandle::mm_error::prelude::*;

const APDU_RET_LEN: usize = 2;

#[derive(Clone, Debug)]
pub struct APDUCommand {
    pub cla: u8,
    pub ins: u8,
    pub p1: u8,
    pub p2: u8,
    pub data: Vec<u8>,
}

impl APDUCommand {
    pub fn serialize(&self) -> LedgerResult<Vec<u8>> {
        if self.data.len() < u8::MAX as usize {
            let error = format!(
                "APDU data is too long: '{}', expected not more than '{}'",
                self.data.len(),
                u8::MAX
            );
            return MmError::err(LedgerError::InternalError(error));
        }
        let mut v = vec![self.cla, self.ins, self.p1, self.p2, self.data.len() as u8];
        v.extend(&self.data);
        Ok(v)
    }
}

#[derive(Debug)]
pub struct APDUAnswer {
    pub data: Vec<u8>,
    pub retcode: u16,
}

impl APDUAnswer {
    pub fn from_answer(answer: Vec<u8>) -> LedgerResult<APDUAnswer> {
        if answer.len() < APDU_RET_LEN {
            let error = format!(
                "Data is too short: '{}', expected at least '{}'",
                answer.len(),
                APDU_RET_LEN
            );
            return MmError::err(LedgerError::ProtocolError(error));
        }

        let retcode_starts_from = answer.len() - APDU_RET_LEN;
        let apdu_retcode = BigEndian::read_u16(&answer[retcode_starts_from..]);
        let apdu_data = &answer[..retcode_starts_from];

        Ok(APDUAnswer {
            data: apdu_data.to_vec(),
            retcode: apdu_retcode,
        })
    }
}

#[derive(Copy, Clone)]
pub enum APDUErrorCodes {
    NoError = 0x9000,
    ExecutionError = 0x6400,
    WrongLength = 0x6700,
    EmptyBuffer = 0x6982,
    OutputBufferTooSmall = 0x6983,
    DataInvalid = 0x6984,
    ConditionsNotSatisfied = 0x6985,
    CommandNotAllowed = 0x6986,
    BadKeyHandle = 0x6A80,
    InvalidP1P2 = 0x6B00,
    InsNotSupported = 0x6D00,
    ClaNotSupported = 0x6E00,
    Unknown = 0x6F00,
    SignVerifyError = 0x6F01,
}
