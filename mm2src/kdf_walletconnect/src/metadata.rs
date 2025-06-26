use std::time::Duration;

use relay_rpc::rpc::params::Metadata;

pub(crate) const RELAY_ADDRESS: &str = "wss://relay.walletconnect.com";
pub(crate) const PROJECT_ID: &str = "86e916bcbacee7f98225dde86b697f5b";
pub(crate) const AUTH_TOKEN_SUB: &str = "http://127.0.0.1:3000";
pub(crate) const AUTH_TOKEN_DURATION: Duration = Duration::from_secs(5 * 60 * 60);
pub(crate) const APP_NAME: &str = "Komodefi Framework";
pub(crate) const APP_DESCRIPTION: &str = "WallectConnect Komodefi Framework Playground";

#[inline]
pub(crate) fn generate_metadata() -> Metadata {
    Metadata {
        description: APP_DESCRIPTION.to_owned(),
        url: AUTH_TOKEN_SUB.to_owned(),
        icons: vec!["https://avatars.githubusercontent.com/u/21276113?s=200&v=4".to_owned()],
        name: APP_NAME.to_owned(),
    }
}
