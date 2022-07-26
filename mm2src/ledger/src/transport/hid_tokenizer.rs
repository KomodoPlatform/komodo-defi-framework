//! Inspired by https://github.com/LedgerHQ/ledgerjs/blob/v6.9.0/packages/devices/src/hid-framing.ts#L27
//! TODO consider moving the `HidTokenizer` implementation into `HidProtocol`.

use crate::transport::apdu::APDUCommand;
use byteorder::{BigEndian, ByteOrder};

/// https://github.com/LedgerHQ/ledgerjs/blob/v6.9.0/packages/devices/src/hid-framing.ts#L10
const LEDGER_PACKET_TAG: u8 = 0x05;
const CHUNK_SIZE: usize = 64;

pub type HidChunk = Vec<u8>;

pub struct HidTokenizer {
    channel: u16,
    chunk_size: usize,
    tag: u8,
}

struct ChunkHeader {
    channel: u16,
    tag: u8,
    chunk_idx: u16,
}

impl ChunkHeader {
    const CHUNK_HEADER_LEN: usize = 5;

    /// |  1  |  2  |  3  |  4  |  5  | .... |
    /// |  CHANNEL  | TAG | CHUNK_IDX | DATA |
    fn serialize(self) -> Vec<u8> {
        let mut data = vec![0; 5];
        BigEndian::write_u16(&mut data[0..2], self.channel);
        data[2] = self.tag;
        BigEndian::write_u16(&mut data[3..5], self.chunk_idx);
        data
    }
}

impl HidTokenizer {
    pub fn new(channel: u16, chunk_size: usize, tag: u8) -> HidTokenizer {
        HidTokenizer {
            channel,
            chunk_size,
            tag,
        }
    }

    pub fn apdu_into_chunks(&self, apdu: APDUCommand) -> Vec<HidChunk> {
        let serialized_apdu = apdu.serialize();
        assert!(serialized_apdu.len() < u16::MAX as usize);

        let mut packet_data = vec![0; 2];
        BigEndian::write_u16(&mut packet_data[0..2], serialized_apdu.len() as u16);
        packet_data.extend(serialized_apdu);

        let chunk_data_len = self.chunk_size - ChunkHeader::CHUNK_HEADER_LEN;
        let chunks_number = ceiling_div(packet_data.len(), chunk_data_len);

        // Fill the packet data with padding.
        // https://github.com/LedgerHQ/ledgerjs/blob/v6.9.0/packages/devices/src/hid-framing.ts#L33
        packet_data.extend(vec![0; chunks_number * chunk_data_len - packet_data.len()]);

        packet_data
            .chunks(chunk_data_len)
            .enumerate()
            .map(|(chunk_idx, chunk_data)| {
                let header = ChunkHeader {
                    channel: self.channel,
                    tag: self.tag,
                    chunk_idx: chunk_idx as u16,
                };
                let mut chunk = header.serialize();
                chunk.extend(chunk_data);
                chunk
            })
            .collect()
    }
}

fn ceiling_div(num: usize, denom: usize) -> usize {
    assert_ne!(denom, 0);
    if (num % denom) == 0 {
        num / denom
    } else {
        num / denom + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Any magic number.
    const CHANNEL: u16 = 111;

    #[test]
    fn test_hid_tokenizer_apdu_into_chunks() {
        let tokenizer = HidTokenizer::new(CHANNEL, CHUNK_SIZE, LEDGER_PACKET_TAG);

        let data = vec![
            121, 156, 246, 65, 161, 98, 102, 231, 56, 96, 173, 91, 158, 103, 121, 157, 76, 106, 41, 217, 23, 190, 26,
            151, 164, 107, 196, 12, 111, 146, 160, 169, 169, 94, 52, 118, 55, 106, 179, 51, 185, 106, 100, 141, 98,
            105, 221, 223, 154, 47, 72, 56, 74, 159, 138, 153, 21, 225, 142, 38, 166, 90, 73, 79,
        ];
        let data_len = data.len();
        assert_eq!(data_len, CHUNK_SIZE);
        let apdu = APDUCommand {
            cla: 1,
            ins: 2,
            p1: 3,
            p2: 4,
            data,
        };
        let actual = tokenizer.apdu_into_chunks(apdu);
        #[rustfmt::skip]
        let expected = vec![
            vec![
                // Chunk header
                0, 111, LEDGER_PACKET_TAG, 0, 0,
                // Total data length `APDUCommand::serialize().len()` big ending
                0, 69,
                // First chunk data (APDU header)
                1, 2, 3, 4, 64,
                // First chunk data
                121, 156, 246, 65, 161, 98, 102, 231, 56, 96, 173, 91, 158, 103, 121, 157,
                76, 106, 41, 217, 23, 190, 26, 151, 164, 107, 196, 12, 111, 146, 160, 169, 169, 94, 52,
                118, 55, 106, 179, 51, 185, 106, 100, 141, 98, 105, 221, 223, 154, 47, 72, 56,
            ],
            vec![
                // Second chunk data (APDU header)
                0, 111, LEDGER_PACKET_TAG, 0, 1,
                // Second chunk data
                74, 159, 138, 153, 21, 225, 142, 38, 166, 90, 73, 79, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0,
            ]
        ];
        assert_eq!(actual, expected);

        // ================= //

        let data = vec![
            121, 156, 246, 65, 161, 98, 102, 231, 56, 96, 173, 91, 158, 103, 121, 157, 76, 106, 41, 217, 23, 190, 26,
            151, 164, 107, 196, 12, 111, 146, 160, 169, 169, 94, 52, 118, 55, 106, 179, 51, 185, 106, 100, 141, 98,
            105, 221, 223, 154, 47, 72, 56,
        ];
        let data_len = data.len();
        assert_eq!(data_len, 52);
        let apdu = APDUCommand {
            cla: 0,
            ins: 255,
            p1: 0,
            p2: 255,
            data,
        };
        let actual = tokenizer.apdu_into_chunks(apdu);
        #[rustfmt::skip]
        let expected = vec![
            vec![
                // Chunk header
                0, 111, LEDGER_PACKET_TAG, 0, 0,
                // Total data length `APDUCommand::serialize().len()` big ending
                0, 57,
                // First chunk data (APDU header)
                0, 255, 0, 255, 52,
                // First chunk data
                121, 156, 246, 65, 161, 98, 102, 231, 56, 96, 173, 91, 158, 103, 121, 157,
                76, 106, 41, 217, 23, 190, 26, 151, 164, 107, 196, 12, 111, 146, 160, 169, 169, 94, 52,
                118, 55, 106, 179, 51, 185, 106, 100, 141, 98, 105, 221, 223, 154, 47, 72, 56,
            ],
        ];
        assert_eq!(actual, expected);

        // ================= //

        let apdu = APDUCommand {
            cla: 0,
            ins: 255,
            p1: 0,
            p2: 255,
            data: Vec::new(),
        };
        let actual = tokenizer.apdu_into_chunks(apdu);
        #[rustfmt::skip]
        let expected = vec![
            vec![
                // Chunk header
                0, 111, LEDGER_PACKET_TAG, 0, 0,
                // Total data length `APDUCommand::serialize().len()` big ending
                0, 5,
                // First chunk data (APDU header)
                0, 255, 0, 255, 0,
                // First chunk data
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
        ];
        assert_eq!(actual, expected);
    }
}
