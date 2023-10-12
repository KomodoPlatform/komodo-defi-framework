use mm2_err_handle::prelude::*;

use crate::proto::messages_bitcoin as proto_bitcoin;
use crate::utxo::{ScriptPubkey, Signature};
use crate::{TrezorError, TrezorResult};

/// Missing fields:
/// * decred_tree - only for Decred
pub struct PrevTxInput {
    /// Hash of previous transaction output to spend by this input.
    pub prev_hash: Vec<u8>,
    /// Index of previous output to spend.
    pub prev_index: u32,
    /// Script signature.
    pub script_sig: Signature,
    /// Sequence.
    pub sequence: u32,
}

impl PrevTxInput {
    fn to_proto(&self) -> proto_bitcoin::TxAckPrevInput {
        let input = proto_bitcoin::PrevInput {
            prev_hash: self.prev_hash.clone(),
            prev_index: self.prev_index,
            script_sig: self.script_sig.clone(),
            sequence: self.sequence,
            decred_tree: None,
        };

        let tx = proto_bitcoin::tx_ack_prev_input::TxAckPrevInputWrapper { input };
        proto_bitcoin::TxAckPrevInput { tx }
    }
}

/// Missing fields:
/// * decred_script_version - only for Decred
pub struct PrevTxOutput {
    /// Amount sent to this output.
    pub amount: u64,
    /// Script Pubkey of this output.
    pub script_pubkey: ScriptPubkey,
}

impl PrevTxOutput {
    fn to_proto(&self) -> proto_bitcoin::TxAckPrevOutput {
        let output = proto_bitcoin::PrevOutput {
            amount: self.amount,
            script_pubkey: self.script_pubkey.clone(),
            decred_script_version: None,
        };

        let tx = proto_bitcoin::tx_ack_prev_output::TxAckPrevOutputWrapper { output };
        proto_bitcoin::TxAckPrevOutput { tx }
    }
}

/// Missing fields:
/// * timestamp - only for Peercoin
pub struct PrevTx {
    /// Transaction inputs.
    pub inputs: Vec<PrevTxInput>,
    /// Transaction outputs.
    pub outputs: Vec<PrevTxOutput>,
    /// Transaction version.
    pub version: u32,
    /// Transaction lock_time.
    pub lock_time: u32,
    /// Expiry height. Only for Decred and Zcash.
    pub expiry: Option<u32>,
    /// only for Zcash, nVersionGroupId.
    pub version_group_id: Option<u32>,
    /// only for Zcash, BRANCH_ID.
    pub branch_id: Option<u32>,
    /// only for Zcash, BRANCH_ID.
    pub extra_data: Vec<u8>,
}

impl PrevTx {
    pub(crate) fn meta_message(&self) -> proto_bitcoin::TxAckPrevMeta {
        let extra_data_len = if self.extra_data.is_empty() {
            None
        } else {
            Some(self.extra_data.len() as u32)
        };

        let tx = proto_bitcoin::PrevTx {
            version: self.version,
            lock_time: self.lock_time,
            inputs_count: self.inputs.len() as u32,
            outputs_count: self.outputs.len() as u32,
            extra_data_len,
            expiry: self.expiry,
            version_group_id: self.version_group_id,
            timestamp: None,
            branch_id: self.branch_id,
        };

        proto_bitcoin::TxAckPrevMeta { tx }
    }

    pub(crate) fn input_message(&self, input_index: usize) -> TrezorResult<proto_bitcoin::TxAckPrevInput> {
        match self.inputs.get(input_index) {
            Some(prev_input) => Ok(prev_input.to_proto()),
            None => {
                let error = format!(
                    "Unexpected index '{}' of the prev-tx input. Actual count of inputs: {}",
                    input_index,
                    self.inputs.len()
                );
                MmError::err(TrezorError::ProtocolError(error))
            },
        }
    }

    pub(crate) fn output_message(&self, output_index: usize) -> TrezorResult<proto_bitcoin::TxAckPrevOutput> {
        match self.outputs.get(output_index) {
            Some(prev_output) => Ok(prev_output.to_proto()),
            None => {
                let error = format!(
                    "Unexpected index '{}' of the prev-tx output. Actual count of outputs: {}",
                    output_index,
                    self.outputs.len()
                );
                MmError::err(TrezorError::ProtocolError(error))
            },
        }
    }

    pub(crate) fn extra_data_message(
        &self,
        offset: usize,
        len: usize,
    ) -> TrezorResult<proto_bitcoin::TxAckPrevExtraData> {
        if self.extra_data.len() < offset + len {
            let error = format!(
                "Unexpected extra-data request: actual len '{}', offset '{}', requested len '{}'",
                self.extra_data.len(),
                offset,
                len
            );
            return MmError::err(TrezorError::ProtocolError(error));
        }
        let extra_data_chunk = self.extra_data[offset..offset + len].to_vec();

        let tx = proto_bitcoin::tx_ack_prev_extra_data::TxAckPrevExtraDataWrapper { extra_data_chunk };
        Ok(proto_bitcoin::TxAckPrevExtraData { tx })
    }
}
