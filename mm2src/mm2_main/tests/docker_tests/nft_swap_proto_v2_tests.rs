use super::eth_docker_tests::{erc721_contract, eth_coin_with_random_privkey, global_nft_with_random_privkey,
                              nft_swap_contract};
use coins::eth::EthCoin;
use coins::nft::nft_structs::{Chain, ContractType};
use coins::{CoinAssocTypes, MakerNftSwapOpsV2, SendNftMakerPaymentArgs, SwapOps, ToBytes};
use common::{block_on, now_sec};
use mm2_number::BigUint;

#[test]
fn send_and_spend_erc721_maker_payment() {
    // TODO generate pair of utxo & eth coins from same random secret for maker / taker
    let maker_global_nft = global_nft_with_random_privkey(nft_swap_contract());
    // in prod we will need to enable global NFT for taker or add new field (for nft swap address) in EthCoin,
    // as EtomicSwapNft will have its own contract address, due to EIP-170 contract size limitations.
    // TODO need to add NFT conf in coin conf and refactor enable nft a bit
    let taker_eth_coin = eth_coin_with_random_privkey(nft_swap_contract());

    let time_lock = now_sec() - 100;
    let taker_pubkey = taker_eth_coin.derive_htlc_pubkey(&[]);

    let send_payment_args: SendNftMakerPaymentArgs<EthCoin> = SendNftMakerPaymentArgs {
        time_lock,
        taker_secret_hash: &[0; 32],
        maker_secret_hash: &[0; 32],
        amount: 1.into(),
        taker_pub: &taker_eth_coin.parse_pubkey(&taker_pubkey).unwrap(),
        swap_unique_data: &[],
        token_address: &erc721_contract().to_bytes(),
        token_id: &BigUint::from(1u32).to_bytes(),
        chain: &Chain::Eth.to_bytes(),
        contract_type: &ContractType::Erc721.to_bytes(),
        swap_contract_address: &nft_swap_contract().to_bytes(),
    };
    block_on(maker_global_nft.send_nft_maker_payment_v2(send_payment_args)).unwrap();
}

#[test]
fn send_and_spend_erc1155_maker_payment() {
    let maker_global_nft = global_nft_with_random_privkey(nft_swap_contract());
    let taker_eth_coin = eth_coin_with_random_privkey(nft_swap_contract());

    let time_lock = now_sec() - 100;
    let taker_pubkey = taker_eth_coin.derive_htlc_pubkey(&[]);

    let send_payment_args: SendNftMakerPaymentArgs<EthCoin> = SendNftMakerPaymentArgs {
        time_lock,
        taker_secret_hash: &[0; 32],
        maker_secret_hash: &[0; 32],
        amount: 3.into(),
        taker_pub: &taker_eth_coin.parse_pubkey(&taker_pubkey).unwrap(),
        swap_unique_data: &[],
        token_address: &erc721_contract().to_bytes(),
        token_id: &BigUint::from(2u32).to_bytes(),
        chain: &Chain::Eth.to_bytes(),
        contract_type: &ContractType::Erc1155.to_bytes(),
        swap_contract_address: &nft_swap_contract().to_bytes(),
    };
    block_on(maker_global_nft.send_nft_maker_payment_v2(send_payment_args)).unwrap();
}
