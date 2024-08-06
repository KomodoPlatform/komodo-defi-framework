use super::eth::{wei_from_big_decimal, EthCoin, EthCoinType, SignedEthTx, TAKER_SWAP_V2};
use super::{SendTakerFundingArgs, Transaction, TransactionErr};
use enum_derives::EnumFromStringify;
use ethabi::Token;
use ethcore_transaction::Action;
use ethereum_types::{Address, Public, U256};
use ethkey::public_to_address;
use futures::compat::Future01CompatExt;
use std::convert::TryInto;

struct TakerFundingArgs {
    dex_fee: U256,
    payment_amount: U256,
    maker_address: Address,
    taker_secret_hash: [u8; 32],
    maker_secret_hash: [u8; 32],
    funding_time_lock: u32,
    payment_time_lock: u32,
}

impl EthCoin {
    pub(crate) async fn send_taker_funding_impl(
        &self,
        args: SendTakerFundingArgs<'_>,
    ) -> Result<SignedEthTx, TransactionErr> {
        let taker_swap_v2_contract = self
            .swap_v2_contracts
            .as_ref()
            .map(|contracts| contracts.taker_swap_v2_contract)
            .ok_or_else(|| TransactionErr::Plain(ERRL!("Expected swap_v2_contracts to be Some, but found None")))?;
        // TODO add burnFee support
        let dex_fee = try_tx_s!(wei_from_big_decimal(&args.dex_fee.fee_amount().into(), self.decimals));

        let payment_amount = try_tx_s!(wei_from_big_decimal(
            &(args.trading_amount.clone() + args.premium_amount.clone()),
            self.decimals
        ));
        let funding_args = {
            let maker_address = public_to_address(&Public::from_slice(args.maker_pub));
            let funding_time_lock: u32 = try_tx_s!(args.funding_time_lock.try_into());
            let payment_time_lock: u32 = try_tx_s!(args.payment_time_lock.try_into());
            TakerFundingArgs {
                dex_fee,
                payment_amount,
                maker_address,
                taker_secret_hash: try_tx_s!(args.taker_secret_hash.try_into()),
                maker_secret_hash: try_tx_s!(args.maker_secret_hash.try_into()),
                funding_time_lock,
                payment_time_lock,
            }
        };
        match &self.coin_type {
            EthCoinType::Eth => {
                let data = try_tx_s!(self.prepare_taker_eth_funding_data(&funding_args).await);
                let eth_total_payment = dex_fee + payment_amount;
                self.sign_and_send_transaction(
                    eth_total_payment,
                    Action::Call(taker_swap_v2_contract),
                    data,
                    // TODO need new consts and params for v2 calls. now it uses v1
                    U256::from(self.gas_limit.eth_payment),
                )
                .compat()
                .await
            },
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => {
                let allowed = self
                    .allowance(taker_swap_v2_contract)
                    .compat()
                    .await
                    .map_err(|e| TransactionErr::Plain(ERRL!("{}", e)))?;
                let data = try_tx_s!(self.prepare_taker_erc20_funding_data(&funding_args, *token_addr).await);
                if allowed < payment_amount {
                    let approved_tx = self.approve(taker_swap_v2_contract, U256::max_value()).compat().await?;
                    self.wait_for_required_allowance(taker_swap_v2_contract, payment_amount, args.funding_time_lock)
                        .compat()
                        .await
                        .map_err(|e| {
                            TransactionErr::Plain(ERRL!(
                                "Allowed value was not updated in time after sending approve transaction {:02x}: {}",
                                approved_tx.tx_hash_as_bytes(),
                                e
                            ))
                        })?;
                }
                self.sign_and_send_transaction(
                    U256::from(0),
                    Action::Call(taker_swap_v2_contract),
                    data,
                    // TODO need new consts and params for v2 calls. now it uses v1
                    U256::from(self.gas_limit.erc20_payment),
                )
                .compat()
                .await
            },
            EthCoinType::Nft { .. } => Err(TransactionErr::ProtocolNotSupported(
                "NFT protocol is not supported for ETH and ERC20 Swaps".to_string(),
            )),
        }
    }

    async fn prepare_taker_eth_funding_data(&self, args: &TakerFundingArgs) -> Result<Vec<u8>, PrepareTxDataError> {
        let function = TAKER_SWAP_V2.function("ethTakerPayment")?;
        let id = self.etomic_swap_id(args.payment_time_lock, &args.maker_secret_hash);
        let data = function.encode_input(&[
            Token::FixedBytes(id),
            Token::Uint(args.dex_fee),
            Token::Address(args.maker_address),
            Token::FixedBytes(args.taker_secret_hash.to_vec()),
            Token::FixedBytes(args.maker_secret_hash.to_vec()),
            Token::Uint(args.funding_time_lock.into()),
            Token::Uint(args.payment_time_lock.into()),
        ])?;
        Ok(data)
    }

    async fn prepare_taker_erc20_funding_data(
        &self,
        args: &TakerFundingArgs,
        token_addr: Address,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        let function = TAKER_SWAP_V2.function("erc20TakerPayment")?;
        let id = self.etomic_swap_id(args.payment_time_lock, &args.maker_secret_hash);
        let data = function.encode_input(&[
            Token::FixedBytes(id),
            Token::Uint(args.payment_amount),
            Token::Uint(args.dex_fee),
            Token::Address(token_addr),
            Token::Address(args.maker_address),
            Token::FixedBytes(args.taker_secret_hash.to_vec()),
            Token::FixedBytes(args.maker_secret_hash.to_vec()),
            Token::Uint(args.funding_time_lock.into()),
            Token::Uint(args.payment_time_lock.into()),
        ])?;
        Ok(data)
    }
}

#[allow(dead_code)]
#[derive(Debug, Display, EnumFromStringify)]
enum PrepareTxDataError {
    #[from_stringify("ethabi::Error")]
    #[display(fmt = "Abi error: {}", _0)]
    AbiError(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}
