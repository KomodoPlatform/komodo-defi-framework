use crate::eth::web3_transport::Web3Transport;
use crate::eth::{EthCoin, ERC20_CONTRACT};
use ethabi::Token;
use ethereum_types::Address;
use futures_util::TryFutureExt;
use web3::types::{BlockId, BlockNumber, CallRequest};
use web3::{Transport, Web3};

async fn call_erc20_function<T: Transport>(
    web3: &Web3<T>,
    token_addr: Address,
    function_name: &str,
) -> Result<Vec<Token>, String> {
    let function = try_s!(ERC20_CONTRACT.function(function_name));
    let data = try_s!(function.encode_input(&[]));
    let request = CallRequest {
        from: Some(Address::default()),
        to: Some(token_addr),
        gas: None,
        gas_price: None,
        value: Some(0.into()),
        data: Some(data.into()),
        ..CallRequest::default()
    };

    let res = web3
        .eth()
        .call(request, Some(BlockId::Number(BlockNumber::Latest)))
        .map_err(|e| ERRL!("{}", e))
        .await?;
    function.decode_output(&res.0).map_err(|e| ERRL!("{}", e))
}

pub(crate) async fn get_token_decimals(web3: &Web3<Web3Transport>, token_addr: Address) -> Result<u8, String> {
    let tokens = call_erc20_function(web3, token_addr, "decimals").await?;
    match tokens[0] {
        Token::Uint(dec) => Ok(dec.as_u64() as u8),
        _ => ERR!("Invalid decimals type {:?}", tokens),
    }
}

async fn get_token_symbol(coin: &EthCoin, token_addr: Address) -> Result<String, String> {
    let web3 = try_s!(coin.web3().await);
    let tokens = call_erc20_function(&web3, token_addr, "symbol").await?;
    match &tokens[0] {
        Token::String(symbol) => Ok(symbol.clone()),
        _ => ERR!("Invalid symbol type {:?}", tokens),
    }
}

#[derive(Serialize)]
pub struct Erc20CustomTokenInfo {
    pub ticker: String,
    pub decimals: u8,
}

pub(crate) async fn get_erc20_token_info(coin: &EthCoin, token_addr: Address) -> Result<Erc20CustomTokenInfo, String> {
    let symbol = get_token_symbol(coin, token_addr).await?;
    let web3 = try_s!(coin.web3().await);
    let decimals = get_token_decimals(&web3, token_addr).await?;
    Ok(Erc20CustomTokenInfo {
        ticker: symbol,
        decimals,
    })
}
