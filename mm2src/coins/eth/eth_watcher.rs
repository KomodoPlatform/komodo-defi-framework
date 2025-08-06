use super::*;

#[async_trait]
impl WatcherOps for EthCoin {
    fn send_maker_payment_spend_preimage(&self, input: SendMakerPaymentSpendPreimageInput) -> TransactionFut {
        Box::new(
            self.watcher_spends_hash_time_locked_payment(input)
                .map(TransactionEnum::from),
        )
    }

    fn create_maker_payment_spend_preimage(
        &self,
        maker_payment_tx: &[u8],
        _time_lock: u64,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        let tx: UnverifiedTransactionWrapper = try_tx_fus!(rlp::decode(maker_payment_tx));
        let signed = try_tx_fus!(SignedEthTx::new(tx));
        let fut = async move { Ok(TransactionEnum::from(signed)) };

        Box::new(fut.boxed().compat())
    }

    fn create_taker_payment_refund_preimage(
        &self,
        taker_payment_tx: &[u8],
        _time_lock: u64,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        let tx: UnverifiedTransactionWrapper = try_tx_fus!(rlp::decode(taker_payment_tx));
        let signed = try_tx_fus!(SignedEthTx::new(tx));
        let fut = async move { Ok(TransactionEnum::from(signed)) };

        Box::new(fut.boxed().compat())
    }

    fn send_taker_payment_refund_preimage(&self, args: RefundPaymentArgs) -> TransactionFut {
        Box::new(
            self.watcher_refunds_hash_time_locked_payment(args)
                .map(TransactionEnum::from),
        )
    }

    fn watcher_validate_taker_fee(&self, validate_fee_args: WatcherValidateTakerFeeInput) -> ValidatePaymentFut<()> {
        validate_fee_impl(
            self.clone(),
            EthValidateFeeArgs {
                fee_tx_hash: &H256::from_slice(validate_fee_args.taker_fee_hash.as_slice()),
                expected_sender: &validate_fee_args.sender_pubkey,
                amount: &BigDecimal::from(0),
                min_block_number: validate_fee_args.min_block_number,
                uuid: &[],
            },
        )

        // TODO: Add validations specific for watchers
        // 1.Validate if taker fee is old
    }

    fn taker_validates_payment_spend_or_refund(&self, input: ValidateWatcherSpendInput) -> ValidatePaymentFut<()> {
        let watcher_reward = try_f!(input
            .watcher_reward
            .clone()
            .ok_or_else(|| ValidatePaymentError::WatcherRewardError("Watcher reward not found".to_string())));
        let expected_reward_amount = try_f!(u256_from_big_decimal(&watcher_reward.amount, ETH_DECIMALS).map_mm_err());

        let expected_swap_contract_address = try_f!(input
            .swap_contract_address
            .try_to_address()
            .map_to_mm(ValidatePaymentError::InvalidParameter)
            .map_mm_err());

        let unsigned: UnverifiedTransactionWrapper = try_f!(rlp::decode(&input.payment_tx));
        let tx = try_f!(SignedEthTx::new(unsigned)
            .map_to_mm(|err| ValidatePaymentError::TxDeserializationError(err.to_string()))
            .map_mm_err());

        let selfi = self.clone();
        let time_lock = try_f!(input
            .time_lock
            .try_into()
            .map_to_mm(ValidatePaymentError::TimelockOverflow)
            .map_mm_err());
        let swap_id = selfi.etomic_swap_id(time_lock, &input.secret_hash);
        let decimals = self.decimals;
        let secret_hash = if input.secret_hash.len() == 32 {
            ripemd160(&input.secret_hash).to_vec()
        } else {
            input.secret_hash.to_vec()
        };
        let maker_addr = try_f!(addr_from_raw_pubkey(&input.maker_pub)
            .map_to_mm(ValidatePaymentError::InvalidParameter)
            .map_mm_err());

        let trade_amount = try_f!(u256_from_big_decimal(&(input.amount), decimals).map_mm_err());
        let fut = async move {
            match tx.unsigned().action() {
                Call(contract_address) => {
                    if *contract_address != expected_swap_contract_address {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Transaction {contract_address:?} was sent to wrong address, expected {expected_swap_contract_address:?}",
                        )));
                    }
                },
                Create => {
                    return MmError::err(ValidatePaymentError::WrongPaymentTx(
                        "Tx action must be Call, found Create instead".to_string(),
                    ));
                },
            };

            let actual_status = selfi
                .payment_status(expected_swap_contract_address, Token::FixedBytes(swap_id.clone()))
                .compat()
                .await
                .map_to_mm(ValidatePaymentError::Transport)
                .map_mm_err()?;
            let expected_status = match input.spend_type {
                WatcherSpendType::MakerPaymentSpend => U256::from(PaymentState::Spent as u8),
                WatcherSpendType::TakerPaymentRefund => U256::from(PaymentState::Refunded as u8),
            };
            if actual_status != expected_status {
                return MmError::err(ValidatePaymentError::UnexpectedPaymentState(format!(
                    "Payment state is not {expected_status}, got {actual_status}"
                )));
            }

            let function_name = match input.spend_type {
                WatcherSpendType::MakerPaymentSpend => get_function_name("receiverSpend", true),
                WatcherSpendType::TakerPaymentRefund => get_function_name("senderRefund", true),
            };
            let function = SWAP_CONTRACT
                .function(&function_name)
                .map_to_mm(|err| ValidatePaymentError::InternalError(err.to_string()))?;

            let decoded = decode_contract_call(function, tx.unsigned().data())
                .map_to_mm(|err| ValidatePaymentError::TxDeserializationError(err.to_string()))?;

            let swap_id_input = get_function_input_data(&decoded, function, 0)
                .map_to_mm(ValidatePaymentError::TxDeserializationError)
                .map_mm_err()?;
            if swap_id_input != Token::FixedBytes(swap_id.clone()) {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "Transaction invalid swap_id arg {:?}, expected {:?}",
                    swap_id_input,
                    Token::FixedBytes(swap_id.clone())
                )));
            }

            let hash_input = match input.spend_type {
                WatcherSpendType::MakerPaymentSpend => {
                    let secret_input = get_function_input_data(&decoded, function, 2)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?
                        .into_fixed_bytes()
                        .ok_or_else(|| {
                            ValidatePaymentError::WrongPaymentTx("Invalid type for secret hash argument".to_string())
                        })?;
                    dhash160(&secret_input).to_vec()
                },
                WatcherSpendType::TakerPaymentRefund => get_function_input_data(&decoded, function, 2)
                    .map_to_mm(ValidatePaymentError::TxDeserializationError)
                    .map_mm_err()?
                    .into_fixed_bytes()
                    .ok_or_else(|| {
                        ValidatePaymentError::WrongPaymentTx("Invalid type for secret argument".to_string())
                    })?,
            };
            if hash_input != secret_hash {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "Transaction secret or secret_hash arg {:?} is invalid, expected {:?}",
                    hash_input,
                    Token::FixedBytes(secret_hash),
                )));
            }

            let my_address = selfi.derivation_method.single_addr_or_err().await.map_mm_err()?;
            let sender_input = get_function_input_data(&decoded, function, 4)
                .map_to_mm(ValidatePaymentError::TxDeserializationError)
                .map_mm_err()?;
            let expected_sender = match input.spend_type {
                WatcherSpendType::MakerPaymentSpend => maker_addr,
                WatcherSpendType::TakerPaymentRefund => my_address,
            };
            if sender_input != Token::Address(expected_sender) {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "Transaction sender arg {:?} is invalid, expected {:?}",
                    sender_input,
                    Token::Address(expected_sender)
                )));
            }

            let receiver_input = get_function_input_data(&decoded, function, 5)
                .map_to_mm(ValidatePaymentError::TxDeserializationError)?;
            let expected_receiver = match input.spend_type {
                WatcherSpendType::MakerPaymentSpend => my_address,
                WatcherSpendType::TakerPaymentRefund => maker_addr,
            };
            if receiver_input != Token::Address(expected_receiver) {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "Transaction receiver arg {:?} is invalid, expected {:?}",
                    receiver_input,
                    Token::Address(expected_receiver)
                )));
            }

            let reward_target_input = get_function_input_data(&decoded, function, 6)
                .map_to_mm(ValidatePaymentError::TxDeserializationError)
                .map_mm_err()?;
            if reward_target_input != Token::Uint(U256::from(watcher_reward.reward_target as u8)) {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "Transaction reward target arg {:?} is invalid, expected {:?}",
                    reward_target_input,
                    Token::Uint(U256::from(watcher_reward.reward_target as u8))
                )));
            }

            let contract_reward_input = get_function_input_data(&decoded, function, 7)
                .map_to_mm(ValidatePaymentError::TxDeserializationError)
                .map_mm_err()?;
            if contract_reward_input != Token::Bool(watcher_reward.send_contract_reward_on_spend) {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "Transaction sends contract reward on spend arg {:?} is invalid, expected {:?}",
                    contract_reward_input,
                    Token::Bool(watcher_reward.send_contract_reward_on_spend)
                )));
            }

            let reward_amount_input = get_function_input_data(&decoded, function, 8)
                .map_to_mm(ValidatePaymentError::TxDeserializationError)
                .map_mm_err()?;
            if reward_amount_input != Token::Uint(expected_reward_amount) {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "Transaction watcher reward amount arg {:?} is invalid, expected {:?}",
                    reward_amount_input,
                    Token::Uint(expected_reward_amount)
                )));
            }

            if tx.unsigned().value() != U256::zero() {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "Transaction value arg {:?} is invalid, expected 0",
                    tx.unsigned().value()
                )));
            }

            match &selfi.coin_type {
                EthCoinType::Eth => {
                    let amount_input = get_function_input_data(&decoded, function, 1)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?;
                    let total_amount = match input.spend_type {
                        WatcherSpendType::MakerPaymentSpend => {
                            if !matches!(watcher_reward.reward_target, RewardTarget::None)
                                || watcher_reward.send_contract_reward_on_spend
                            {
                                trade_amount + expected_reward_amount
                            } else {
                                trade_amount
                            }
                        },
                        WatcherSpendType::TakerPaymentRefund => trade_amount + expected_reward_amount,
                    };
                    if amount_input != Token::Uint(total_amount) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Transaction amount arg {:?} is invalid, expected {:?}",
                            amount_input,
                            Token::Uint(total_amount),
                        )));
                    }

                    let token_address_input = get_function_input_data(&decoded, function, 3)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?;
                    if token_address_input != Token::Address(Address::default()) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Transaction token address arg {:?} is invalid, expected {:?}",
                            token_address_input,
                            Token::Address(Address::default()),
                        )));
                    }
                },
                EthCoinType::Erc20 {
                    platform: _,
                    token_addr,
                } => {
                    let amount_input = get_function_input_data(&decoded, function, 1)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?;
                    if amount_input != Token::Uint(trade_amount) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Transaction amount arg {:?} is invalid, expected {:?}",
                            amount_input,
                            Token::Uint(trade_amount),
                        )));
                    }

                    let token_address_input = get_function_input_data(&decoded, function, 3)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?;
                    if token_address_input != Token::Address(*token_addr) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Transaction token address arg {:?} is invalid, expected {:?}",
                            token_address_input,
                            Token::Address(*token_addr),
                        )));
                    }
                },
                EthCoinType::Nft { .. } => {
                    return MmError::err(ValidatePaymentError::ProtocolNotSupported(
                        "Nft protocol is not supported by watchers yet".to_string(),
                    ))
                },
            }

            Ok(())
        };
        Box::new(fut.boxed().compat())
    }

    fn watcher_validate_taker_payment(&self, input: WatcherValidatePaymentInput) -> ValidatePaymentFut<()> {
        let unsigned: UnverifiedTransactionWrapper = try_f!(rlp::decode(&input.payment_tx));
        let tx = try_f!(SignedEthTx::new(unsigned)
            .map_to_mm(|err| ValidatePaymentError::TxDeserializationError(err.to_string()))
            .map_mm_err());
        let sender = try_f!(addr_from_raw_pubkey(&input.taker_pub)
            .map_to_mm(ValidatePaymentError::InvalidParameter)
            .map_mm_err());
        let receiver = try_f!(addr_from_raw_pubkey(&input.maker_pub)
            .map_to_mm(ValidatePaymentError::InvalidParameter)
            .map_mm_err());
        let time_lock = try_f!(input
            .time_lock
            .try_into()
            .map_to_mm(ValidatePaymentError::TimelockOverflow)
            .map_mm_err());

        let selfi = self.clone();
        let swap_id = selfi.etomic_swap_id(time_lock, &input.secret_hash);
        let secret_hash = if input.secret_hash.len() == 32 {
            ripemd160(&input.secret_hash).to_vec()
        } else {
            input.secret_hash.to_vec()
        };
        let expected_swap_contract_address = self.swap_contract_address;
        let fallback_swap_contract = self.fallback_swap_contract;

        let fut = async move {
            let tx_from_rpc = selfi.transaction(TransactionId::Hash(tx.tx_hash())).await?;

            let tx_from_rpc = tx_from_rpc.as_ref().ok_or_else(|| {
                ValidatePaymentError::TxDoesNotExist(format!("Didn't find provided tx {tx:?} on ETH node"))
            })?;

            if tx_from_rpc.from != Some(sender) {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "{INVALID_SENDER_ERR_LOG}: Payment tx {tx_from_rpc:?} was sent from wrong address, expected {sender:?}"
                )));
            }

            let swap_contract_address = tx_from_rpc.to.ok_or_else(|| {
                ValidatePaymentError::TxDeserializationError(format!(
                    "Swap contract address not found in payment Tx {tx_from_rpc:?}"
                ))
            })?;

            if swap_contract_address != expected_swap_contract_address
                && Some(swap_contract_address) != fallback_swap_contract
            {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "{INVALID_CONTRACT_ADDRESS_ERR_LOG}: Payment tx {tx_from_rpc:?} was sent to wrong address, expected either {expected_swap_contract_address:?} or the fallback {fallback_swap_contract:?}"
                )));
            }

            let status = selfi
                .payment_status(swap_contract_address, Token::FixedBytes(swap_id.clone()))
                .compat()
                .await
                .map_to_mm(ValidatePaymentError::Transport)
                .map_mm_err()?;
            if status != U256::from(PaymentState::Sent as u8) && status != U256::from(PaymentState::Spent as u8) {
                return MmError::err(ValidatePaymentError::UnexpectedPaymentState(format!(
                    "{INVALID_PAYMENT_STATE_ERR_LOG}: Payment state is not PAYMENT_STATE_SENT or PAYMENT_STATE_SPENT, got {status}"
                )));
            }

            let watcher_reward = selfi
                .get_taker_watcher_reward(&input.maker_coin, None, None, None, input.wait_until)
                .await
                .map_err(|err| ValidatePaymentError::WatcherRewardError(err.into_inner().to_string()))?;
            let expected_reward_amount = u256_from_big_decimal(&watcher_reward.amount, ETH_DECIMALS).map_mm_err()?;

            match &selfi.coin_type {
                EthCoinType::Eth => {
                    let function_name = get_function_name("ethPayment", true);
                    let function = SWAP_CONTRACT
                        .function(&function_name)
                        .map_to_mm(|err| ValidatePaymentError::InternalError(err.to_string()))
                        .map_mm_err()?;
                    let decoded = decode_contract_call(function, &tx_from_rpc.input.0)
                        .map_to_mm(|err| ValidatePaymentError::TxDeserializationError(err.to_string()))
                        .map_mm_err()?;

                    let swap_id_input = get_function_input_data(&decoded, function, 0)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?;
                    if swap_id_input != Token::FixedBytes(swap_id.clone()) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "{INVALID_SWAP_ID_ERR_LOG}: Invalid 'swap_id' {decoded:?}, expected {swap_id:?}"
                        )));
                    }

                    let receiver_input = get_function_input_data(&decoded, function, 1)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?;
                    if receiver_input != Token::Address(receiver) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "{INVALID_RECEIVER_ERR_LOG}: Payment tx receiver arg {receiver_input:?} is invalid, expected {:?}", Token::Address(receiver)
                        )));
                    }

                    let secret_hash_input = get_function_input_data(&decoded, function, 2)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?;
                    if secret_hash_input != Token::FixedBytes(secret_hash.to_vec()) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx secret_hash arg {:?} is invalid, expected {:?}",
                            secret_hash_input,
                            Token::FixedBytes(secret_hash.to_vec()),
                        )));
                    }

                    let time_lock_input = get_function_input_data(&decoded, function, 3)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?;
                    if time_lock_input != Token::Uint(U256::from(input.time_lock)) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx time_lock arg {:?} is invalid, expected {:?}",
                            time_lock_input,
                            Token::Uint(U256::from(input.time_lock)),
                        )));
                    }

                    let reward_target_input = get_function_input_data(&decoded, function, 4)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?;
                    let expected_reward_target = watcher_reward.reward_target as u8;
                    if reward_target_input != Token::Uint(U256::from(expected_reward_target)) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx reward target arg {reward_target_input:?} is invalid, expected {expected_reward_target:?}"
                        )));
                    }

                    let sends_contract_reward_input = get_function_input_data(&decoded, function, 5)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?;
                    if sends_contract_reward_input != Token::Bool(watcher_reward.send_contract_reward_on_spend) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx sends_contract_reward_on_spend arg {:?} is invalid, expected {:?}",
                            sends_contract_reward_input, watcher_reward.send_contract_reward_on_spend
                        )));
                    }

                    let reward_amount_input = get_function_input_data(&decoded, function, 6)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?
                        .into_uint()
                        .ok_or_else(|| {
                            ValidatePaymentError::WrongPaymentTx("Invalid type for reward amount argument".to_string())
                        })?;

                    validate_watcher_reward(expected_reward_amount.as_u64(), reward_amount_input.as_u64(), false)
                        .map_mm_err()?;

                    // TODO: Validate the value
                },
                EthCoinType::Erc20 {
                    platform: _,
                    token_addr,
                } => {
                    let function_name = get_function_name("erc20Payment", true);
                    let function = SWAP_CONTRACT
                        .function(&function_name)
                        .map_to_mm(|err| ValidatePaymentError::InternalError(err.to_string()))
                        .map_mm_err()?;
                    let decoded = decode_contract_call(function, &tx_from_rpc.input.0)
                        .map_to_mm(|err| ValidatePaymentError::TxDeserializationError(err.to_string()))
                        .map_mm_err()?;

                    let swap_id_input = get_function_input_data(&decoded, function, 0)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?;
                    if swap_id_input != Token::FixedBytes(swap_id.clone()) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "{INVALID_SWAP_ID_ERR_LOG}: Invalid 'swap_id' {decoded:?}, expected {swap_id:?}"
                        )));
                    }

                    let token_addr_input = get_function_input_data(&decoded, function, 2)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?;
                    if token_addr_input != Token::Address(*token_addr) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx token_addr arg {:?} is invalid, expected {:?}",
                            token_addr_input,
                            Token::Address(*token_addr)
                        )));
                    }

                    let receiver_addr_input = get_function_input_data(&decoded, function, 3)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?;
                    if receiver_addr_input != Token::Address(receiver) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "{INVALID_RECEIVER_ERR_LOG}: Payment tx receiver arg {receiver_addr_input:?} is invalid, expected {:?}", Token::Address(receiver),
                        )));
                    }

                    let secret_hash_input = get_function_input_data(&decoded, function, 4)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?;
                    if secret_hash_input != Token::FixedBytes(secret_hash.to_vec()) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx secret_hash arg {:?} is invalid, expected {:?}",
                            secret_hash_input,
                            Token::FixedBytes(secret_hash.to_vec()),
                        )));
                    }

                    let time_lock_input = get_function_input_data(&decoded, function, 5)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?;
                    if time_lock_input != Token::Uint(U256::from(input.time_lock)) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx time_lock arg {:?} is invalid, expected {:?}",
                            time_lock_input,
                            Token::Uint(U256::from(input.time_lock)),
                        )));
                    }

                    let reward_target_input = get_function_input_data(&decoded, function, 6)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?;
                    let expected_reward_target = watcher_reward.reward_target as u8;
                    if reward_target_input != Token::Uint(U256::from(expected_reward_target)) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx reward target arg {reward_target_input:?} is invalid, expected {expected_reward_target:?}"
                        )));
                    }

                    let sends_contract_reward_input = get_function_input_data(&decoded, function, 7)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?;
                    if sends_contract_reward_input != Token::Bool(watcher_reward.send_contract_reward_on_spend) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx sends_contract_reward_on_spend arg {:?} is invalid, expected {:?}",
                            sends_contract_reward_input, watcher_reward.send_contract_reward_on_spend
                        )));
                    }

                    let reward_amount_input = get_function_input_data(&decoded, function, 8)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)
                        .map_mm_err()?
                        .into_uint()
                        .ok_or_else(|| {
                            ValidatePaymentError::WrongPaymentTx("Invalid type for reward amount argument".to_string())
                        })?;

                    validate_watcher_reward(expected_reward_amount.as_u64(), reward_amount_input.as_u64(), false)
                        .map_mm_err()?;

                    if tx_from_rpc.value != reward_amount_input {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx value arg {:?} is invalid, expected {:?}",
                            tx_from_rpc.value, reward_amount_input
                        )));
                    }
                },
                EthCoinType::Nft { .. } => {
                    return MmError::err(ValidatePaymentError::ProtocolNotSupported(
                        "Nft protocol is not supported by watchers yet".to_string(),
                    ))
                },
            }

            Ok(())
        };
        Box::new(fut.boxed().compat())
    }

    async fn watcher_search_for_swap_tx_spend(
        &self,
        input: WatcherSearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        let unverified: UnverifiedTransactionWrapper = try_s!(rlp::decode(input.tx));
        let tx = try_s!(SignedEthTx::new(unverified));
        let swap_contract_address = match tx.unsigned().action() {
            Call(address) => *address,
            Create => return Err(ERRL!("Invalid payment action: the payment action cannot be create")),
        };

        self.search_for_swap_tx_spend(
            input.tx,
            swap_contract_address,
            input.secret_hash,
            input.search_from_block,
            true,
        )
        .await
    }

    async fn get_taker_watcher_reward(
        &self,
        other_coin: &MmCoinEnum,
        _coin_amount: Option<BigDecimal>,
        _other_coin_amount: Option<BigDecimal>,
        reward_amount: Option<BigDecimal>,
        wait_until: u64,
    ) -> Result<WatcherReward, MmError<WatcherRewardError>> {
        let reward_target = if other_coin.is_eth() {
            RewardTarget::Contract
        } else {
            RewardTarget::PaymentSender
        };

        let amount = match reward_amount {
            Some(amount) => amount,
            None => self.get_watcher_reward_amount(wait_until).await?,
        };

        let send_contract_reward_on_spend = false;

        Ok(WatcherReward {
            amount,
            is_exact_amount: false,
            reward_target,
            send_contract_reward_on_spend,
        })
    }

    async fn get_maker_watcher_reward(
        &self,
        other_coin: &MmCoinEnum,
        reward_amount: Option<BigDecimal>,
        wait_until: u64,
    ) -> Result<Option<WatcherReward>, MmError<WatcherRewardError>> {
        let reward_target = if other_coin.is_eth() {
            RewardTarget::None
        } else {
            RewardTarget::PaymentSpender
        };

        let is_exact_amount = reward_amount.is_some();
        let amount = match reward_amount {
            Some(amount) => amount,
            None => {
                let gas_cost_eth = self.get_watcher_reward_amount(wait_until).await?;

                match &self.coin_type {
                    EthCoinType::Eth => gas_cost_eth,
                    EthCoinType::Erc20 { .. } => {
                        if other_coin.is_eth() {
                            gas_cost_eth
                        } else {
                            get_base_price_in_rel(Some(self.ticker().to_string()), Some("ETH".to_string()))
                                .await
                                .and_then(|price_in_eth| gas_cost_eth.checked_div(price_in_eth))
                                .ok_or_else(|| {
                                    WatcherRewardError::RPCError(format!(
                                        "Price of coin {} in ETH could not be found",
                                        self.ticker()
                                    ))
                                })?
                        }
                    },
                    EthCoinType::Nft { .. } => {
                        return MmError::err(WatcherRewardError::InternalError(
                            "Nft Protocol is not supported yet!".to_string(),
                        ))
                    },
                }
            },
        };

        let send_contract_reward_on_spend = other_coin.is_eth();

        Ok(Some(WatcherReward {
            amount,
            is_exact_amount,
            reward_target,
            send_contract_reward_on_spend,
        }))
    }
}
