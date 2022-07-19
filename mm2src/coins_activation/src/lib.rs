mod bch_with_tokens_activation;
mod context;
mod erc20_token_activation;
mod eth_with_token_activation;
mod l2;
#[cfg(not(target_arch = "wasm32"))] mod lightning_activation;
mod platform_coin_with_tokens;
mod prelude;
mod slp_token_activation;
#[cfg(not(target_arch = "wasm32"))]
mod solana_with_tokens_activation;
#[cfg(not(target_arch = "wasm32"))] mod spl_token_activation;
mod standalone_coin;
mod token;
mod utxo_activation;
#[cfg(not(target_arch = "wasm32"))] mod z_coin_activation;

pub use l2::enable_l2;
pub use platform_coin_with_tokens::enable_platform_coin_with_tokens;
pub use standalone_coin::{init_standalone_coin, init_standalone_coin_status, init_standalone_coin_user_action};
pub use token::enable_token;
