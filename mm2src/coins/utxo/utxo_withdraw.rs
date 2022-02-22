use crate::init_withdraw::{WithdrawAwaitingStatus, WithdrawInProgressStatus, WithdrawTaskHandle};
use crate::utxo::utxo_common::{big_decimal_from_sat, UtxoTxBuilder};
use crate::utxo::{output_script, sat_from_big_decimal, ActualTxFee, Address, FeePolicy, PrivKeyPolicy,
                  UtxoAddressFormat, UtxoCoinFields, UtxoCommonOps, UtxoFeeDetails, UtxoTx, UTXO_LOCK};
use crate::{GetWithdrawSenderAddress, MarketCoinOps, TransactionDetails, WithdrawError, WithdrawFee, WithdrawRequest,
            WithdrawResult};
use async_trait::async_trait;
use chain::TransactionOutput;
use common::log::info;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::now_ms;
use crypto::hw_rpc_task::{HwConnectStatuses, TrezorRpcTaskConnectProcessor};
use crypto::trezor::client::TrezorClient;
use crypto::trezor::{TrezorError, TrezorProcessingError};
use crypto::{Bip32Error, CryptoCtx, CryptoInitError, DerivationPath, HardwareWalletArc, HwError, HwProcessingError,
             HwWalletType};
use keys::{Public as PublicKey, Type as ScriptType};
use rpc_task::RpcTaskError;
use script::{Builder, Script, SignatureVersion, TransactionInputSigner};
use serialization::{serialize, serialize_with_flags, SERIALIZE_TRANSACTION_WITNESS};
use std::iter::once;
use std::time::Duration;
use utxo_signer::sign_params::{SendingOutputInfo, SpendingInputInfo, UtxoSignTxParamsBuilder};
use utxo_signer::{with_key_pair, UtxoSignTxError};
use utxo_signer::{SignPolicy, UtxoSignerOps};

const TREZOR_CONNECT_TIMEOUT: Duration = Duration::from_secs(300);
const TREZOR_PIN_TIMEOUT: Duration = Duration::from_secs(300);

impl From<UtxoSignTxError> for WithdrawError {
    fn from(sign_err: UtxoSignTxError) -> Self {
        match sign_err {
            UtxoSignTxError::TrezorError(trezor) => WithdrawError::from(trezor),
            UtxoSignTxError::Transport(transport) => WithdrawError::Transport(transport),
            UtxoSignTxError::Internal(internal) => WithdrawError::InternalError(internal),
            sign_err => WithdrawError::InternalError(sign_err.to_string()),
        }
    }
}

impl From<HwProcessingError<RpcTaskError>> for WithdrawError {
    fn from(e: HwProcessingError<RpcTaskError>) -> Self {
        match e {
            HwProcessingError::HwError(hw) => WithdrawError::from(hw),
            HwProcessingError::ProcessorError(rpc_task) => WithdrawError::from(rpc_task),
        }
    }
}

impl From<TrezorProcessingError<RpcTaskError>> for WithdrawError {
    fn from(e: TrezorProcessingError<RpcTaskError>) -> Self {
        match e {
            TrezorProcessingError::TrezorError(trezor) => WithdrawError::from(trezor),
            TrezorProcessingError::ProcessorError(rpc_task) => WithdrawError::from(rpc_task),
        }
    }
}

impl From<HwError> for WithdrawError {
    fn from(e: HwError) -> Self {
        let error = e.to_string();
        match e {
            HwError::NoTrezorDeviceAvailable => WithdrawError::NoTrezorDeviceAvailable,
            HwError::FoundUnexpectedDevice { .. } => WithdrawError::FoundUnexpectedDevice(error),
            _ => WithdrawError::HardwareWalletInternal(error),
        }
    }
}

impl From<TrezorError> for WithdrawError {
    fn from(e: TrezorError) -> Self { WithdrawError::HardwareWalletInternal(e.to_string()) }
}

impl From<CryptoInitError> for WithdrawError {
    fn from(e: CryptoInitError) -> Self { WithdrawError::InternalError(e.to_string()) }
}

impl From<RpcTaskError> for WithdrawError {
    fn from(e: RpcTaskError) -> Self {
        let error = e.to_string();
        match e {
            RpcTaskError::Canceled => WithdrawError::InternalError("Canceled".to_owned()),
            RpcTaskError::Timeout(timeout) => WithdrawError::Timeout(timeout),
            RpcTaskError::NoSuchTask(_) | RpcTaskError::UnexpectedTaskStatus { .. } => {
                WithdrawError::InternalError(error)
            },
            RpcTaskError::Internal(internal) => WithdrawError::InternalError(internal),
        }
    }
}

impl From<Bip32Error> for WithdrawError {
    fn from(e: Bip32Error) -> Self {
        WithdrawError::HardwareWalletInternal(format!("Error parsing pubkey received from Hardware Wallet: {}", e))
    }
}

#[async_trait]
pub trait UtxoWithdraw<Coin>
where
    Self: Sized + Sync,
    Coin: AsRef<UtxoCoinFields> + UtxoCommonOps + MarketCoinOps + Send + Sync + 'static,
{
    fn coin(&self) -> &Coin;

    fn from_address(&self) -> Address;

    fn from_address_string(&self) -> String;

    fn request(&self) -> &WithdrawRequest;

    fn signature_version(&self) -> SignatureVersion {
        match self.from_address().addr_format {
            UtxoAddressFormat::Segwit => SignatureVersion::WitnessV0,
            _ => self.coin().as_ref().conf.signature_version,
        }
    }

    fn prev_script(&self) -> Script { Builder::build_p2pkh(&self.from_address().hash) }

    fn on_generating_transaction(&self) -> Result<(), MmError<WithdrawError>>;

    fn on_finishing(&self) -> Result<(), MmError<WithdrawError>>;

    async fn sign_tx(&self, unsigned_tx: TransactionInputSigner) -> Result<UtxoTx, MmError<WithdrawError>>;

    async fn build(self) -> WithdrawResult {
        let coin = self.coin();
        let decimals = coin.as_ref().decimals;
        let conf = &self.coin().as_ref().conf;
        let req = self.request();

        let to = coin
            .address_from_str(&req.to)
            .map_to_mm(WithdrawError::InvalidAddress)?;

        let is_p2pkh = to.prefix == conf.pub_addr_prefix && to.t_addr_prefix == conf.pub_t_addr_prefix;
        let is_p2sh = to.prefix == conf.p2sh_addr_prefix && to.t_addr_prefix == conf.p2sh_t_addr_prefix && conf.segwit;

        let script_type = if is_p2pkh {
            ScriptType::P2PKH
        } else if is_p2sh {
            ScriptType::P2SH
        } else {
            return MmError::err(WithdrawError::InvalidAddress("Expected either P2PKH or P2SH".into()));
        };

        // Generate unsigned transaction.
        self.on_generating_transaction()?;

        let script_pubkey = output_script(&to, script_type).to_bytes();

        let _utxo_lock = UTXO_LOCK.lock().await;
        let (unspents, _) = coin.list_unspent_ordered(&self.from_address()).await?;
        let (value, fee_policy) = if req.max {
            (
                unspents.iter().fold(0, |sum, unspent| sum + unspent.value),
                FeePolicy::DeductFromOutput(0),
            )
        } else {
            let value = sat_from_big_decimal(&req.amount, decimals)?;
            (value, FeePolicy::SendExact)
        };
        let outputs = vec![TransactionOutput { value, script_pubkey }];

        let mut tx_builder = UtxoTxBuilder::new(coin)
            .with_from_address(self.from_address())
            .add_available_inputs(unspents)
            .add_outputs(outputs)
            .with_fee_policy(fee_policy);

        match req.fee {
            Some(WithdrawFee::UtxoFixed { ref amount }) => {
                let fixed = sat_from_big_decimal(amount, decimals)?;
                tx_builder = tx_builder.with_fee(ActualTxFee::FixedPerKb(fixed));
            },
            Some(WithdrawFee::UtxoPerKbyte { ref amount }) => {
                let dynamic = sat_from_big_decimal(amount, decimals)?;
                tx_builder = tx_builder.with_fee(ActualTxFee::Dynamic(dynamic));
            },
            Some(ref fee_policy) => {
                let error = format!(
                    "Expected 'UtxoFixed' or 'UtxoPerKbyte' fee types, found {:?}",
                    fee_policy
                );
                return MmError::err(WithdrawError::InvalidFeePolicy(error));
            },
            None => (),
        };
        let (unsigned, data) = tx_builder.build().await.mm_err(|gen_tx_error| {
            WithdrawError::from_generate_tx_error(gen_tx_error, coin.ticker().to_owned(), decimals)
        })?;

        // Sign the `unsigned` transaction.
        let signed = self.sign_tx(unsigned).await?;

        // Finish by generating `TransactionDetails` from the signed transaction.
        self.on_finishing()?;

        let fee_amount = data.fee_amount + data.unused_change.unwrap_or_default();
        let fee_details = UtxoFeeDetails {
            coin: Some(self.coin().as_ref().conf.ticker.clone()),
            amount: big_decimal_from_sat(fee_amount as i64, decimals),
        };
        let tx_hex = match coin.addr_format() {
            UtxoAddressFormat::Segwit => serialize_with_flags(&signed, SERIALIZE_TRANSACTION_WITNESS).into(),
            _ => serialize(&signed).into(),
        };
        Ok(TransactionDetails {
            from: vec![self.from_address_string()],
            to: vec![req.to.clone()],
            total_amount: big_decimal_from_sat(data.spent_by_me as i64, decimals),
            spent_by_me: big_decimal_from_sat(data.spent_by_me as i64, decimals),
            received_by_me: big_decimal_from_sat(data.received_by_me as i64, decimals),
            my_balance_change: big_decimal_from_sat(data.received_by_me as i64 - data.spent_by_me as i64, decimals),
            tx_hash: signed.hash().reversed().to_vec().into(),
            tx_hex,
            fee_details: Some(fee_details.into()),
            block_height: 0,
            coin: coin.as_ref().conf.ticker.clone(),
            internal_id: vec![].into(),
            timestamp: now_ms() / 1000,
            kmd_rewards: data.kmd_rewards,
            transaction_type: Default::default(),
        })
    }
}

pub struct InitUtxoWithdraw<'a, Coin> {
    coin: Coin,
    task_handle: &'a WithdrawTaskHandle,
    req: WithdrawRequest,
    from_address: Address,
    /// Displayed [`InitUtxoWithdraw::from_address`].
    from_address_string: String,
    /// Derivation path from which [`InitUtxoWithdraw::from_address`] was derived.
    from_derivation_path: DerivationPath,
    /// Public key corresponding to [`InitUtxoWithdraw::from_address`].
    from_pubkey: PublicKey,
    trezor: Option<TrezorClient>,
}

#[async_trait]
impl<'a, Coin> UtxoWithdraw<Coin> for InitUtxoWithdraw<'a, Coin>
where
    Coin: AsRef<UtxoCoinFields> + UtxoCommonOps + MarketCoinOps + UtxoSignerOps + Send + Sync + 'static,
{
    fn coin(&self) -> &Coin { &self.coin }

    fn from_address(&self) -> Address { self.from_address.clone() }

    fn from_address_string(&self) -> String { self.from_address_string.clone() }

    fn request(&self) -> &WithdrawRequest { &self.req }

    fn on_generating_transaction(&self) -> Result<(), MmError<WithdrawError>> {
        let amount_display = if self.req.max {
            "MAX".to_owned()
        } else {
            self.req.amount.to_string()
        };

        // Display the address from which we are trying to withdraw funds.
        info!(
            "Trying to withdraw {} {} from {} to {}",
            amount_display, self.req.coin, self.from_address_string, self.req.to,
        );

        Ok(self
            .task_handle
            .update_in_progress_status(WithdrawInProgressStatus::GeneratingTransaction)?)
    }

    fn on_finishing(&self) -> Result<(), MmError<WithdrawError>> {
        Ok(self
            .task_handle
            .update_in_progress_status(WithdrawInProgressStatus::Finishing)?)
    }

    async fn sign_tx(&self, unsigned_tx: TransactionInputSigner) -> Result<UtxoTx, MmError<WithdrawError>> {
        self.task_handle
            .update_in_progress_status(WithdrawInProgressStatus::SigningTransaction)?;

        let mut sign_params = UtxoSignTxParamsBuilder::new();

        // TODO refactor [`UtxoTxBuilder::build`] to return `SpendingInputInfo` and `SendingOutputInfo` within `AdditionalTxData`.
        sign_params.add_inputs_infos(unsigned_tx.inputs.iter().map(|_input| SpendingInputInfo::P2PKH {
            address_derivation_path: self.from_derivation_path.clone(),
            address_pubkey: self.from_pubkey,
        }));
        sign_params.add_outputs_infos(once(SendingOutputInfo {
            destination_address: self.req.to.clone(),
        }));
        match unsigned_tx.outputs.len() {
            // There is no change output.
            1 => (),
            // There is a change output.
            2 => {
                sign_params.add_outputs_infos(once(SendingOutputInfo {
                    destination_address: self.from_address_string.clone(),
                }));
            },
            unexpected => {
                let error = format!("Unexpected number of outputs: {}", unexpected);
                return MmError::err(WithdrawError::InternalError(error));
            },
        }

        sign_params
            .with_signature_version(self.signature_version())
            .with_unsigned_tx(unsigned_tx)
            .with_prev_script(Builder::build_p2pkh(&self.from_address.hash));
        let sign_params = sign_params.build()?;

        let sign_policy = match self.coin.as_ref().priv_key_policy {
            PrivKeyPolicy::KeyPair(ref key_pair) => SignPolicy::WithKeyPair(key_pair),
            PrivKeyPolicy::HardwareWallet => match self.trezor {
                Some(ref trezor) => SignPolicy::WithTrezor(trezor.clone()),
                None => {
                    let error = "'InitUtxoWithdraw::trezor' is expected to be set".to_owned();
                    return MmError::err(WithdrawError::InternalError(error));
                },
            },
        };

        // TODO refactor [`UtxoSignerOps::sign_tx`] to use [`TrezorInteractWithUser::interact_with_user_if_required`].
        self.task_handle
            .update_in_progress_status(WithdrawInProgressStatus::WaitingForUserToConfirmSigning)?;
        let signed = self.coin.sign_tx(sign_params, sign_policy).await?;

        Ok(signed)
    }
}

impl<'a, Coin> InitUtxoWithdraw<'a, Coin>
where
    Coin: AsRef<UtxoCoinFields>
        + UtxoCommonOps
        + MarketCoinOps
        + UtxoSignerOps
        + GetWithdrawSenderAddress<Address = Address, Pubkey = PublicKey>,
{
    pub async fn new(
        ctx: MmArc,
        coin: Coin,
        req: WithdrawRequest,
        task_handle: &'a WithdrawTaskHandle,
    ) -> Result<InitUtxoWithdraw<'a, Coin>, MmError<WithdrawError>> {
        let crypto_ctx = CryptoCtx::from_ctx(&ctx)?;
        match *crypto_ctx {
            CryptoCtx::KeyPair(_) => Self::new_with_key_pair(coin, req, task_handle).await,
            CryptoCtx::HardwareWallet(ref hw_ctx) => match hw_ctx.hw_wallet_type() {
                HwWalletType::Trezor => Self::new_with_trezor(hw_ctx, coin, req, task_handle).await,
            },
        }
    }

    async fn new_with_key_pair(
        coin: Coin,
        req: WithdrawRequest,
        task_handle: &'a WithdrawTaskHandle,
    ) -> Result<InitUtxoWithdraw<'a, Coin>, MmError<WithdrawError>> {
        let from = coin.get_withdraw_sender_address(&req).await?;
        let from_address_string = from.address.display_address().map_to_mm(WithdrawError::InternalError)?;
        let from_derivation_path = match from.derivation_path {
            Some(der_path) => der_path,
            // Temporary initialize the derivation path by default since this field is not used without Trezor.
            None => DerivationPath::default(),
        };
        Ok(InitUtxoWithdraw {
            coin,
            task_handle,
            req,
            from_address: from.address,
            from_address_string,
            from_derivation_path,
            from_pubkey: from.pubkey,
            trezor: None,
        })
    }

    async fn new_with_trezor(
        hw_ctx: &HardwareWalletArc,
        coin: Coin,
        req: WithdrawRequest,
        task_handle: &'a WithdrawTaskHandle,
    ) -> Result<InitUtxoWithdraw<'a, Coin>, MmError<WithdrawError>> {
        let from = coin.get_withdraw_sender_address(&req).await?;
        let from_derivation_path = match from.derivation_path {
            Some(der_path) => der_path,
            None => {
                let error = "Cannot determine 'from' address derivation path".to_owned();
                return MmError::err(WithdrawError::UnexpectedFromAddress(error));
            },
        };
        let from_address_string = from.address.display_address().map_to_mm(WithdrawError::InternalError)?;

        let trezor_connect_processor = TrezorRpcTaskConnectProcessor::new(task_handle, HwConnectStatuses {
            on_connect: WithdrawInProgressStatus::WaitingForTrezorToConnect,
            on_connected: WithdrawInProgressStatus::Preparing,
            on_connection_failed: WithdrawInProgressStatus::Finishing,
            on_button_request: WithdrawInProgressStatus::WaitingForUserToConfirmPubkey,
            on_pin_request: WithdrawAwaitingStatus::WaitForTrezorPin,
            on_ready: WithdrawInProgressStatus::Preparing,
        })
        .with_connect_timeout(TREZOR_CONNECT_TIMEOUT)
        .with_pin_timeout(TREZOR_PIN_TIMEOUT);

        let trezor_client = hw_ctx.trezor(&trezor_connect_processor).await?;

        Ok(InitUtxoWithdraw {
            coin,
            task_handle,
            req,
            from_address: from.address,
            from_address_string,
            from_derivation_path,
            from_pubkey: from.pubkey,
            trezor: Some(trezor_client),
        })
    }
}

pub struct StandardUtxoWithdraw<Coin> {
    coin: Coin,
    req: WithdrawRequest,
    my_address: Address,
    my_address_string: String,
}

#[async_trait]
impl<Coin> UtxoWithdraw<Coin> for StandardUtxoWithdraw<Coin>
where
    Coin: AsRef<UtxoCoinFields> + UtxoCommonOps + MarketCoinOps + Send + Sync + 'static,
{
    fn coin(&self) -> &Coin { &self.coin }

    fn from_address(&self) -> Address { self.my_address.clone() }

    fn from_address_string(&self) -> String { self.my_address_string.clone() }

    fn request(&self) -> &WithdrawRequest { &self.req }

    fn on_generating_transaction(&self) -> Result<(), MmError<WithdrawError>> { Ok(()) }

    fn on_finishing(&self) -> Result<(), MmError<WithdrawError>> { Ok(()) }

    async fn sign_tx(&self, unsigned_tx: TransactionInputSigner) -> Result<UtxoTx, MmError<WithdrawError>> {
        let key_pair = self.coin.as_ref().priv_key_policy.key_pair_or_err()?;
        Ok(with_key_pair::sign_tx(
            unsigned_tx,
            key_pair,
            self.prev_script(),
            self.signature_version(),
            self.coin.as_ref().conf.fork_id,
        )?)
    }
}

impl<Coin> StandardUtxoWithdraw<Coin>
where
    Coin: AsRef<UtxoCoinFields> + MarketCoinOps,
{
    pub fn new(coin: Coin, req: WithdrawRequest) -> Result<Self, MmError<WithdrawError>> {
        let my_address = coin.as_ref().derivation_method.iguana_or_err()?.clone();
        let my_address_string = coin.my_address().map_to_mm(WithdrawError::InternalError)?;
        Ok(StandardUtxoWithdraw {
            coin,
            req,
            my_address,
            my_address_string,
        })
    }
}
