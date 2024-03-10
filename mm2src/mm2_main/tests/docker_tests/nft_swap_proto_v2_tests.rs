use super::eth_docker_tests::{eth_coin_with_random_privkey, global_nft_with_random_privkey, nft_swap_contract};
use coins::eth::EthCoin;
use coins::nft::nft_structs::{Chain, ContractType};
use coins::{CoinAssocTypes, SendNftMakerPaymentArgs, SwapOps, ToBytes};
use common::now_sec;
use mm2_number::BigUint;

#[test]
fn send_and_spend_erc721_maker_payment() {
    // TODO generate pair of utxo & eth coins from same random secret for maker / taker
    let _maker_global_nft = global_nft_with_random_privkey(nft_swap_contract());
    // in prod we will need to enable global NFT for taker or add new field (for nft swap address) in EthCoin,
    // as EtomicSwapNft will have its own contract address, due to EIP-170 contract size limitations.
    // TODO need to add NFT conf in coin conf and refactor enable nft a bit
    let taker_eth_coin = eth_coin_with_random_privkey(nft_swap_contract());

    let time_lock = now_sec() - 100;
    let taker_pubkey = taker_eth_coin.derive_htlc_pubkey(&[]);

    let _send_payment_args: SendNftMakerPaymentArgs<EthCoin> = SendNftMakerPaymentArgs {
        time_lock,
        taker_secret_hash: &[],
        maker_secret_hash: &[],
        amount: 1.into(),
        taker_pub: &taker_eth_coin.parse_pubkey(&taker_pubkey).unwrap(),
        swap_unique_data: &[],
        token_address: &[],
        token_id: &BigUint::from(1u32).to_bytes(),
        chain: &Chain::Eth.to_bytes(),
        contract_type: &ContractType::Erc721.to_bytes(),
        swap_contract_address: nft_swap_contract().as_bytes(),
    };
}
