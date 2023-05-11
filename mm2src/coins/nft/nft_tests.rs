use crate::nft;
use nft::{DIRECTION_BOTH_MORALIS, FORMAT_DECIMAL_MORALIS};

const TEST_ENDPOINT: &str = "https://moralis-proxy.komodo.earth/api/v2/";
const TEST_WALLET_ADDR_EVM: &str = "0x394d86994f954ed931b86791b62fe64f4c5dac37";

fn create_nft_list_url() -> String {
    format!("{TEST_ENDPOINT}{TEST_WALLET_ADDR_EVM}/nft?chain=POLYGON&{FORMAT_DECIMAL_MORALIS}")
}

fn create_nft_tx_history_url() -> String {
    format!("{TEST_ENDPOINT}{TEST_WALLET_ADDR_EVM}/nft/transfers?chain=POLYGON&{FORMAT_DECIMAL_MORALIS}&{DIRECTION_BOTH_MORALIS}")
}

fn create_nft_metadata_url() -> String {
    format!(
        "{TEST_ENDPOINT}nft/0xed55e4477b795eaa9bb4bca24df42214e1a05c18/1111777?chain=POLYGON&{FORMAT_DECIMAL_MORALIS}"
    )
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod native_tests {
    use crate::nft::nft_structs::{NftTransferHistoryWrapper, NftWrapper};
    use crate::nft::nft_tests::{create_nft_list_url, create_nft_metadata_url, create_nft_tx_history_url,
                                TEST_WALLET_ADDR_EVM};
    use crate::nft::send_moralis_request;
    use common::block_on;

    #[test]
    fn test_moralis_nft_list() {
        let url = create_nft_list_url();
        let response = block_on(send_moralis_request(url.as_str())).unwrap();
        let nfts_list = response["result"].as_array().unwrap();
        assert_eq!(2, nfts_list.len());
        for nft_json in nfts_list {
            let nft_wrapper: NftWrapper = serde_json::from_str(&nft_json.to_string()).unwrap();
            assert_eq!(TEST_WALLET_ADDR_EVM, nft_wrapper.owner_of)
        }
    }

    #[test]
    fn test_moralis_nft_transfer_history() {
        let url = create_nft_tx_history_url();
        let response = block_on(send_moralis_request(url.as_str())).unwrap();
        let transfer_list = response["result"].as_array().unwrap();
        assert_eq!(2, transfer_list.len());
        for transfer in transfer_list {
            let transfer_wrapper: NftTransferHistoryWrapper = serde_json::from_str(&transfer.to_string()).unwrap();
            assert_eq!(TEST_WALLET_ADDR_EVM, transfer_wrapper.to_address);
        }
    }

    #[test]
    fn test_moralis_nft_metadata() {
        let url = create_nft_metadata_url();
        let response = block_on(send_moralis_request(url.as_str())).unwrap();
        let nft_wrapper: NftWrapper = serde_json::from_str(&response.to_string()).unwrap();
        assert_eq!(41237364, *nft_wrapper.block_number_minted)
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm_tests {
    use crate::nft::nft_structs::{NftTransferHistoryWrapper, NftWrapper};
    use crate::nft::nft_tests::{create_nft_list_url, create_nft_metadata_url, create_nft_tx_history_url,
                                TEST_WALLET_ADDR_EVM};
    use crate::nft::send_moralis_request;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    async fn test_moralis_nft_list() {
        let url = create_nft_list_url();
        let response = send_moralis_request(url.as_str()).await.unwrap();
        let nfts_list = response["result"].as_array().unwrap();
        assert_eq!(2, nfts_list.len());
        for nft_json in nfts_list {
            let nft_wrapper: NftWrapper = serde_json::from_str(&nft_json.to_string()).unwrap();
            assert_eq!(TEST_WALLET_ADDR_EVM, nft_wrapper.owner_of)
        }
    }

    #[wasm_bindgen_test]
    async fn test_moralis_nft_transfer_history() {
        let url = create_nft_tx_history_url();
        let response = send_moralis_request(url.as_str()).await.unwrap();
        let transfer_list = response["result"].as_array().unwrap();
        assert_eq!(2, transfer_list.len());
        for transfer in transfer_list {
            let transfer_wrapper: NftTransferHistoryWrapper = serde_json::from_str(&transfer.to_string()).unwrap();
            assert_eq!(TEST_WALLET_ADDR_EVM, transfer_wrapper.to_address);
        }
    }

    #[wasm_bindgen_test]
    async fn test_moralis_nft_metadata() {
        let url = create_nft_metadata_url();
        let response = send_moralis_request(url.as_str()).await.unwrap();
        let nft_wrapper: NftWrapper = serde_json::from_str(&response.to_string()).unwrap();
        assert_eq!(41237364, *nft_wrapper.block_number_minted)
    }
}
