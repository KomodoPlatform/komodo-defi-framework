use crate::utxo::rpc_clients::ElectrumClient;
use async_trait::async_trait;
use chain::{BlockHeader, RawBlockHeader, Transaction as UtxoTx};
use common::executor::Timer;
use common::log::error;
use common::now_ms;
use keys::hash::H256;
use mm2_err_handle::prelude::*;
use serialization::serialize_list;
use spv_validation::helpers_validation::SPVError;
use spv_validation::spv_proof::{SPVProof, TRY_SPV_PROOF_INTERVAL};

#[derive(Clone)]
pub struct ConfirmedTransactionInfo {
    pub tx: UtxoTx,
    pub header: BlockHeader,
    pub index: u64,
    pub height: u64,
}

#[async_trait]
pub trait SimplePaymentVerification {
    async fn validate_spv_proof(
        &self,
        tx: &UtxoTx,
        try_spv_proof_until: u64,
    ) -> Result<ConfirmedTransactionInfo, MmError<SPVError>>;
}

#[async_trait]
impl SimplePaymentVerification for ElectrumClient {
    // Todo: this is not working right should get a header from DB to use for validation
    async fn validate_spv_proof(
        &self,
        tx: &UtxoTx,
        try_spv_proof_until: u64,
    ) -> Result<ConfirmedTransactionInfo, MmError<SPVError>> {
        if tx.outputs.is_empty() {
            return MmError::err(SPVError::InvalidVout);
        }

        let (merkle_branch, header, height) = loop {
            if now_ms() / 1000 > try_spv_proof_until {
                // Todo: find a way to not show this error when height is still 0
                error!(
                    "Waited too long until {} for transaction {:?} to validate spv proof",
                    try_spv_proof_until,
                    tx.hash().reversed(),
                );
                return MmError::err(SPVError::Timeout);
            }

            // Todo: should get merkle from RPC and header from storage (also check where we get headers every where and get it from storage or RPC)
            match self.get_merkle_and_header_from_rpc(tx).await {
                Ok(res) => break res,
                Err(e) => {
                    error!(
                        "Failed spv proof validation for transaction {} with error: {:?}, retrying in {} seconds.",
                        tx.hash().reversed(),
                        e,
                        TRY_SPV_PROOF_INTERVAL,
                    );

                    Timer::sleep(TRY_SPV_PROOF_INTERVAL as f64).await;
                },
            }
        };

        let raw_header = RawBlockHeader::new(header.raw().take())?;
        let intermediate_nodes: Vec<H256> = merkle_branch
            .merkle
            .into_iter()
            .map(|hash| hash.reversed().into())
            .collect();

        let proof = SPVProof {
            tx_id: tx.hash(),
            vin: serialize_list(&tx.inputs).take(),
            vout: serialize_list(&tx.outputs).take(),
            index: merkle_branch.pos as u64,
            confirming_header: header.clone(),
            raw_header,
            intermediate_nodes,
        };

        // Todo: refactor validate function along validate_spv_proof
        proof.validate().map_err(MmError::new)?;

        Ok(ConfirmedTransactionInfo {
            tx: tx.clone(),
            header,
            index: proof.index,
            height,
        })
    }
}
