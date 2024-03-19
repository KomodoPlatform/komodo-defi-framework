use super::eth_docker_tests::{erc1155_contract, erc721_contract, global_nft_with_random_privkey, nft_swap_contract};
use coins::eth::EthCoin;
use coins::nft::nft_structs::{Chain, ContractType};
use coins::{CoinAssocTypes, ConfirmPaymentInput, MakerNftSwapOpsV2, MarketCoinOps, SendNftMakerPaymentArgs, SwapOps,
            ToBytes, Transaction, ValidateNftMakerPaymentArgs};
use common::{block_on, now_sec};
use futures01::Future;
use mm2_number::BigUint;

#[test]
fn send_and_spend_erc721_maker_payment() {
    // in prod we will need to enable global NFT for taker or add new field (for nft swap address) in EthCoin,
    // as EtomicSwapNft will have its own contract address, due to EIP-170 contract size limitations.
    // TODO need to add NFT conf in coin conf and refactor enable nft a bit

    let maker_global_nft = global_nft_with_random_privkey(nft_swap_contract());
    let taker_global_nft = global_nft_with_random_privkey(nft_swap_contract());

    let time_lock = now_sec() - 100;
    let maker_pubkey = maker_global_nft.derive_htlc_pubkey(&[]);
    let taker_pubkey = taker_global_nft.derive_htlc_pubkey(&[]);

    let send_payment_args: SendNftMakerPaymentArgs<EthCoin> = SendNftMakerPaymentArgs {
        time_lock,
        taker_secret_hash: &[0; 32],
        maker_secret_hash: &[0; 32],
        amount: 1.into(),
        taker_pub: &taker_global_nft.parse_pubkey(&taker_pubkey).unwrap(),
        swap_unique_data: &[],
        token_address: &erc721_contract().to_bytes(),
        token_id: &BigUint::from(1u32).to_bytes(),
        chain: &Chain::Eth.to_bytes(),
        contract_type: &ContractType::Erc721.to_bytes(),
        swap_contract_address: &nft_swap_contract().to_bytes(),
    };
    let maker_payment = block_on(maker_global_nft.send_nft_maker_payment_v2(send_payment_args)).unwrap();

    let confirm_input = ConfirmPaymentInput {
        payment_tx: maker_payment.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    maker_global_nft.wait_for_confirmations(confirm_input).wait().unwrap();

    let validate_args = ValidateNftMakerPaymentArgs {
        maker_payment_tx: &maker_payment,
        time_lock,
        taker_secret_hash: &[0; 32],
        maker_secret_hash: &[0; 32],
        amount: 1.into(),
        taker_pub: &taker_global_nft.parse_pubkey(&taker_pubkey).unwrap(),
        maker_pub: &maker_global_nft.parse_pubkey(&maker_pubkey).unwrap(),
        swap_unique_data: &[],
        token_address: &erc721_contract().to_bytes(),
        token_id: &BigUint::from(1u32).to_bytes(),
        chain: &Chain::Eth.to_bytes(),
        contract_type: &ContractType::Erc721.to_bytes(),
        swap_contract_address: &nft_swap_contract().to_bytes(),
    };
    block_on(maker_global_nft.validate_nft_maker_payment_v2(validate_args)).unwrap();
}

#[test]
fn send_and_spend_erc1155_maker_payment() {
    let maker_global_nft = global_nft_with_random_privkey(nft_swap_contract());
    let taker_global_nft = global_nft_with_random_privkey(nft_swap_contract());

    let time_lock = now_sec() - 100;
    let maker_pubkey = maker_global_nft.derive_htlc_pubkey(&[]);
    let taker_pubkey = taker_global_nft.derive_htlc_pubkey(&[]);

    let send_payment_args: SendNftMakerPaymentArgs<EthCoin> = SendNftMakerPaymentArgs {
        time_lock,
        taker_secret_hash: &[0; 32],
        maker_secret_hash: &[0; 32],
        amount: 3.into(),
        taker_pub: &taker_global_nft.parse_pubkey(&taker_pubkey).unwrap(),
        swap_unique_data: &[],
        token_address: &erc1155_contract().to_bytes(),
        token_id: &BigUint::from(1u32).to_bytes(),
        chain: &Chain::Eth.to_bytes(),
        contract_type: &ContractType::Erc1155.to_bytes(),
        swap_contract_address: &nft_swap_contract().to_bytes(),
    };
    let maker_payment = block_on(maker_global_nft.send_nft_maker_payment_v2(send_payment_args)).unwrap();

    let confirm_input = ConfirmPaymentInput {
        payment_tx: maker_payment.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    maker_global_nft.wait_for_confirmations(confirm_input).wait().unwrap();

    let validate_args = ValidateNftMakerPaymentArgs {
        maker_payment_tx: &maker_payment,
        time_lock,
        taker_secret_hash: &[0; 32],
        maker_secret_hash: &[0; 32],
        amount: 3.into(),
        taker_pub: &taker_global_nft.parse_pubkey(&taker_pubkey).unwrap(),
        maker_pub: &maker_global_nft.parse_pubkey(&maker_pubkey).unwrap(),
        swap_unique_data: &[],
        token_address: &erc1155_contract().to_bytes(),
        token_id: &BigUint::from(1u32).to_bytes(),
        chain: &Chain::Eth.to_bytes(),
        contract_type: &ContractType::Erc1155.to_bytes(),
        swap_contract_address: &nft_swap_contract().to_bytes(),
    };
    block_on(maker_global_nft.validate_nft_maker_payment_v2(validate_args)).unwrap();
}
