use crate::siacoin::{siacoin_from_hastings, siacoin_to_hastings, SiaCoin, SiaFeeDetails, SiaFeePolicy,
                     SiaTransactionTypes, Address, Currency, Keypair, SiacoinElement, SiacoinOutput, SpendPolicy, V2TransactionBuilder};
use crate::{MarketCoinOps, PrivKeyPolicy, TransactionData, TransactionDetails, TransactionType, WithdrawError,
            WithdrawRequest, WithdrawResult};
use common::now_sec;
use mm2_err_handle::mm_error::MmError;
use mm2_err_handle::prelude::*;
use std::str::FromStr;

pub struct SiaWithdrawBuilder<'a> {
    coin: &'a SiaCoin,
    req: WithdrawRequest,
    from_address: Address,
    key_pair: &'a Keypair,
}

impl<'a> SiaWithdrawBuilder<'a> {
    #[allow(clippy::result_large_err)]
    pub fn new(coin: &'a SiaCoin, req: WithdrawRequest) -> Result<Self, MmError<WithdrawError>> {
        let (key_pair, from_address) = match &*coin.priv_key_policy {
            PrivKeyPolicy::Iguana(key_pair) => (key_pair, key_pair.public().address()),
            _ => {
                return Err(WithdrawError::UnsupportedError(
                    "Only Iguana keypair is supported for Sia coin for now!".to_string(),
                )
                .into());
            },
        };

        Ok(SiaWithdrawBuilder {
            coin,
            req,
            from_address,
            key_pair,
        })
    }

    #[allow(clippy::result_large_err)]
    fn select_outputs(
        &self,
        mut unspent_outputs: Vec<SiacoinElement>,
        total_amount: u128,
    ) -> Result<Vec<SiacoinElement>, MmError<WithdrawError>> {
        // Sort outputs from largest to smallest
        unspent_outputs.sort_by(|a, b| b.siacoin_output.value.0.cmp(&a.siacoin_output.value.0));

        let mut selected = Vec::new();
        let mut selected_amount = 0;

        // Select outputs until the total amount is reached
        for output in unspent_outputs {
            selected_amount += *output.siacoin_output.value;
            selected.push(output);

            if selected_amount >= total_amount {
                break;
            }
        }

        if selected_amount < total_amount {
            return Err(MmError::new(WithdrawError::NotSufficientBalance {
                coin: self.coin.ticker().to_string(),
                available: siacoin_from_hastings(selected_amount),
                required: siacoin_from_hastings(total_amount),
            }));
        }

        Ok(selected)
    }

    pub async fn build(self) -> WithdrawResult {
        // Todo: fee estimation based on transaction size
        const TX_FEE_HASTINGS: u128 = 10_000_000_000_000_000_000;

        let to = Address::from_str(&self.req.to).map_err(|e| WithdrawError::InvalidAddress(e.to_string()))?;

        // Calculate the total amount to send (including fee)
        let tx_amount_hastings = siacoin_to_hastings(self.req.amount.clone())?;
        let total_amount = tx_amount_hastings + TX_FEE_HASTINGS;

        // Get unspent outputs
        let unspent_outputs = self
            .coin
            .get_unspent_outputs(self.from_address.clone())
            .await
            .map_err(|e| WithdrawError::Transport(e.to_string()))?;

        // Select outputs to use as inputs
        let selected_outputs = self.select_outputs(unspent_outputs, total_amount)?;

        // Calculate change amount
        let input_sum: u128 = selected_outputs.iter().map(|o| *o.siacoin_output.value).sum();
        let change_amount = input_sum - total_amount;

        // Construct transaction
        let mut tx_builder = V2TransactionBuilder::new();

        // Add inputs
        for output in selected_outputs {
            tx_builder = tx_builder.add_siacoin_input(output, SpendPolicy::PublicKey(self.key_pair.public()));
        }

        // Add output for recipient
        tx_builder = tx_builder.add_siacoin_output(SiacoinOutput {
            value: tx_amount_hastings.into(),
            address: to.clone(),
        });

        // Add change output if necessary
        if change_amount > 0 {
            tx_builder = tx_builder.add_siacoin_output(SiacoinOutput {
                value: change_amount.into(),
                address: self.from_address.clone(),
            });
        }

        // Add miner fee
        tx_builder = tx_builder.miner_fee(Currency::from(TX_FEE_HASTINGS));

        // Sign the transaction
        let signed_tx_builder = tx_builder
            .sign_simple(vec![self.key_pair])
            .map_to_mm(WithdrawError::SigningError)?;

        // Build the final transaction
        let signed_tx = signed_tx_builder.build();

        let spent_by_me = siacoin_from_hastings(input_sum);
        let received_by_me = siacoin_from_hastings(change_amount);

        Ok(TransactionDetails {
            tx: TransactionData::Sia {
                tx_json: SiaTransactionTypes::V2Transaction(signed_tx.clone()),
                tx_hash: signed_tx.txid().to_string(),
            },
            from: vec![self.from_address.to_string()],
            to: vec![self.req.to.clone()],
            total_amount: spent_by_me.clone(),
            spent_by_me: spent_by_me.clone(),
            received_by_me: received_by_me.clone(),
            my_balance_change: received_by_me - spent_by_me,
            fee_details: Some(
                SiaFeeDetails {
                    coin: self.coin.ticker().to_string(),
                    policy: SiaFeePolicy::Fixed,
                    total_amount: siacoin_from_hastings(TX_FEE_HASTINGS),
                }
                .into(),
            ),
            block_height: 0,
            coin: self.coin.ticker().to_string(),
            internal_id: vec![].into(),
            timestamp: now_sec(),
            kmd_rewards: None,
            transaction_type: TransactionType::SiaV2Transaction,
            memo: None,
        })
    }
}
