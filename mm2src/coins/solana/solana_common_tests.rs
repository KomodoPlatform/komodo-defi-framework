use super::*;
use crate::solana::spl::{SplToken, SplTokenFields};
use crypto::privkey::{bip39_seed_from_passphrase, key_pair_from_seed};
use ed25519_dalek_bip32::{DerivationPath, ExtendedSecretKey};
use mm2_core::mm_ctx::MmCtxBuilder;
use solana_client::rpc_client::RpcClient;
use solana_sdk::commitment_config::{CommitmentConfig, CommitmentLevel};
use std::str::FromStr;

pub enum SolanaNet {
    //Mainnet,
    Testnet,
    Devnet,
}

pub fn solana_net_to_url(net_type: SolanaNet) -> String {
    match net_type {
        //SolanaNet::Mainnet => "https://api.mainnet-beta.solana.com".to_string(),
        SolanaNet::Testnet => "https://api.testnet.solana.com/".to_string(),
        SolanaNet::Devnet => "https://api.devnet.solana.com".to_string(),
    }
}

pub fn generate_key_pair_from_seed(seed: &str) -> Keypair {
    let derivation_path = DerivationPath::from_str("m/44'/501'/0'").unwrap();
    let seed = bip39_seed_from_passphrase(seed).unwrap();

    let ext = ExtendedSecretKey::from_seed(&seed.0)
        .unwrap()
        .derive(&derivation_path)
        .unwrap();
    let pub_key = ext.public_key();
    let pair = ed25519_dalek::Keypair {
        secret: ext.secret_key,
        public: pub_key,
    };

    solana_sdk::signature::keypair_from_seed(pair.to_bytes().as_ref()).unwrap()
}

pub fn generate_key_pair_from_iguana_seed(seed: String) -> Keypair {
    let key_pair = key_pair_from_seed(seed.as_str()).unwrap();
    let secret_key = ed25519_dalek::SecretKey::from_bytes(key_pair.private().secret.as_slice()).unwrap();
    let public_key = ed25519_dalek::PublicKey::from(&secret_key);
    let other_key_pair = ed25519_dalek::Keypair {
        secret: secret_key,
        public: public_key,
    };
    solana_sdk::signature::keypair_from_seed(other_key_pair.to_bytes().as_ref()).unwrap()
}

pub fn spl_coin_for_test(
    solana_coin: SolanaCoin,
    ticker: String,
    decimals: u8,
    token_contract_address: Pubkey,
) -> SplToken {
    SplToken {
        conf: Arc::new(SplTokenFields {
            decimals,
            ticker,
            token_contract_address,
            abortable_system: AbortableQueue::default(),
        }),
        platform_coin: solana_coin,
    }
}

pub fn solana_coin_for_test(seed: String, net_type: SolanaNet) -> (MmArc, SolanaCoin) {
    let url = solana_net_to_url(net_type);
    let client = RpcClient::new_with_commitment(url, CommitmentConfig {
        commitment: CommitmentLevel::Finalized,
    });
    let conf = json!({
        "coins":[
           {"coin":"SOL","name":"solana","protocol":{"type":"SOL"},"rpcport":80,"mm2":1}
        ]
    });
    let ctx = MmCtxBuilder::new().with_conf(conf).into_mm_arc();
    let (ticker, decimals) = ("SOL".to_string(), 8);
    let key_pair = generate_key_pair_from_iguana_seed(seed);
    let my_address = key_pair.pubkey().to_string();
    let spl_tokens_infos = Arc::new(Mutex::new(HashMap::new()));
    let spawner = AbortableQueue::default();

    let solana_coin = SolanaCoin(Arc::new(SolanaCoinImpl {
        decimals,
        my_address,
        key_pair,
        ticker,
        client,
        spl_tokens_infos,
        abortable_system: spawner,
    }));
    (ctx, solana_coin)
}
