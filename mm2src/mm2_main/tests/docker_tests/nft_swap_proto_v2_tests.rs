use super::docker_tests_common::{utxo_nft_eth_pair_with_random_privkey, MYCOIN};
use super::eth_docker_tests::{erc721_contract, nft_swap_contract};
use coins::eth::EthCoin;
use coins::nft::nft_structs::{Chain, ContractType};
use coins::{CoinAssocTypes, MakerNftSwapOpsV2, SendNftMakerPaymentArgs, SwapOps, ToBytes};
use common::{block_on, now_sec};
use mm2_number::BigUint;

#[test]
fn send_and_spend_erc721_maker_payment() {
    // in prod we will need to enable global NFT for taker or add new field (for nft swap address) in EthCoin,
    // as EtomicSwapNft will have its own contract address, due to EIP-170 contract size limitations.
    // TODO need to add NFT conf in coin conf and refactor enable nft a bit

    let (_maker_utxo, maker_global_nft) = utxo_nft_eth_pair_with_random_privkey(MYCOIN, nft_swap_contract(), true);
    // We can treat taker global NFT as ETH coin, as they are generated with same priv key and configurations
    let (_taker_utxo, taker_global_nft) = utxo_nft_eth_pair_with_random_privkey(MYCOIN, nft_swap_contract(), false);

    let time_lock = now_sec() - 100;
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
    block_on(maker_global_nft.send_nft_maker_payment_v2(send_payment_args)).unwrap();
}

#[test]
fn send_and_spend_erc1155_maker_payment() {
    let (_maker_utxo, maker_global_nft) = utxo_nft_eth_pair_with_random_privkey(MYCOIN, nft_swap_contract(), true);
    // We can treat taker global NFT as ETH coin
    let (_taker_utxo, taker_global_nft) = utxo_nft_eth_pair_with_random_privkey(MYCOIN, nft_swap_contract(), false);

    let time_lock = now_sec() - 100;
    let taker_pubkey = taker_global_nft.derive_htlc_pubkey(&[]);

    let send_payment_args: SendNftMakerPaymentArgs<EthCoin> = SendNftMakerPaymentArgs {
        time_lock,
        taker_secret_hash: &[0; 32],
        maker_secret_hash: &[0; 32],
        amount: 3.into(),
        taker_pub: &taker_global_nft.parse_pubkey(&taker_pubkey).unwrap(),
        swap_unique_data: &[],
        token_address: &erc721_contract().to_bytes(),
        token_id: &BigUint::from(1u32).to_bytes(),
        chain: &Chain::Eth.to_bytes(),
        contract_type: &ContractType::Erc1155.to_bytes(),
        swap_contract_address: &nft_swap_contract().to_bytes(),
    };
    block_on(maker_global_nft.send_nft_maker_payment_v2(send_payment_args)).unwrap();
}
