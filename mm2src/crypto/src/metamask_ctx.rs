// use crate::metamask_login::{AtomicDEXDomain, AtomicDEXLoginRequest, ADEX_LOGIN_TYPE, ADEX_TYPES};
use mm2_metamask::MetamaskProvider;
use std::ops::Deref;
use std::sync::Arc;

pub use mm2_metamask::{EthAccount, MetamaskError, MetamaskResult};

#[derive(Clone)]
pub struct MetamaskArc(Arc<MetamaskCtx>);

impl MetamaskArc {
    pub fn new(metamask_ctx: MetamaskCtx) -> MetamaskArc { MetamaskArc(Arc::new(metamask_ctx)) }
}

impl Deref for MetamaskArc {
    type Target = MetamaskCtx;

    fn deref(&self) -> &Self::Target { &self.0 }
}

pub struct MetamaskCtx {
    eth_account: EthAccount,
    // eth_account_pubkey: String,
    metamask_provider: MetamaskProvider,
}

impl MetamaskCtx {
    pub async fn init() -> MetamaskResult<MetamaskCtx> {
        let metamask_provider = MetamaskProvider::detect_metamask_provider()?;
        let eth_account = metamask_provider.eth_request_accounts().await?;

        // Uncomment this to finish MetaMask login.
        // TODO figure out how to serialize the source message into bytes and feed it to `ethkey::recover`.
        // HINT: https://github.com/MetaMask/eth-sig-util/blob/d1f01ba799de734d84cdf599d19a215f8fecb5b2/src/sign-typed-data.ts#L449
        // https://github.com/MetaMask/eth-sig-util/blob/d1f01ba799de734d84cdf599d19a215f8fecb5b2/src/sign-typed-data.ts#L551
        //
        // let request = AtomicDEXLoginRequest::new(domain.name.clone());
        // let signature = metamask_provider.sign_typed_data_v4(
        //     eth_account.address.clone(),
        //     &ADEX_TYPES,
        //     domain,
        //     request,
        //     ADEX_LOGIN_TYPE,
        // );

        Ok(MetamaskCtx {
            eth_account,
            metamask_provider,
        })
    }

    pub fn eth_account(&self) -> &EthAccount { &self.eth_account }
}
