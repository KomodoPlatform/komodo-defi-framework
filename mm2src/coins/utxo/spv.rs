use async_trait::async_trait;
use chain::Transaction as UtxoTx;
use common::log::error;
use keys::hash::H256;
use serialization::serialize_list;
use spv_validation::helpers_validation::SPVError;
use spv_validation::spv_proof::{SPVProof, TRY_SPV_PROOF_INTERVAL};

use crate::utxo::rpc_clients::{ConfirmedTransactionInfo, ElectrumClient};

#[async_trait]
pub trait SimplePaymentVerification {
    async fn validate_spv_proof(
        &self,
        tx: &UtxoTx,
        try_spv_proof_until: u64,
    ) -> Result<ConfirmedTransactionInfo, SPVError>;
}

#[async_trait]
impl SimplePaymentVerification for ElectrumClient {
    async fn validate_spv_proof(
        &self,
        tx: &UtxoTx,
        try_spv_proof_until: u64,
    ) -> Result<ConfirmedTransactionInfo, SPVError> {
        if tx.outputs.is_empty() {
            return Err(SPVError::InvalidVout);
        }

        let tx_hash = tx.hash().reversed();
        let (merkle_branch, validated_header, height) =
            retry_on_err!(async { self.get_merkle_and_validated_header(tx).await })
                .repeat_every_secs(TRY_SPV_PROOF_INTERVAL as f64)
                .with_timeout_secs(try_spv_proof_until as f64)
                .inspect_err(move |e| {
                    error!(
                        "Failed spv proof validation for transaction {tx_hash} with error: {e:?}, retrying in {TRY_SPV_PROOF_INTERVAL} seconds.",
                    )
                })
                .await
                .map_err(|_| SPVError::Timeout)?;

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
            intermediate_nodes,
        };

        proof.validate(&validated_header)?;

        Ok(ConfirmedTransactionInfo {
            tx: tx.clone(),
            header: validated_header,
            index: proof.index,
            height,
        })
    }
}
