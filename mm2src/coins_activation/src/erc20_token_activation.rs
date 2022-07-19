use serde::Deserialize;


#[derive(Clone, Debug, Deserialize)]
pub struct Erc20ActivationRequest {
    pub required_confirmations: Option<u64>,
}
