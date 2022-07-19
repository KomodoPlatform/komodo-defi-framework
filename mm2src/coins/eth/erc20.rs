use ethereum_types::Address;

use super::EthCoin;

pub struct Erc20Token {
    pub conf: Arc<SplTokenConf>,
    pub platform_coin: EthCoin,
}
