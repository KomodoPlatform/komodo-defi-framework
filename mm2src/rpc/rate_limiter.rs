use crate::mm2::rpc::DispatcherError;
use common::mm_ctx::from_ctx;
use common::mm_ctx::MmArc;
use common::mm_error::MmError;
use derive_more::Display;
use futures::lock::Mutex as AsyncMutex;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

pub const LIMIT_FAILED_REQUEST: usize = 10;
pub type RateInfosRegistry = HashMap<IpAddr, usize>;

#[derive(Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum RateLimitError {
    #[display(fmt = "Rate limit maximum attempt reached: {}", LIMIT_FAILED_REQUEST)]
    Banned,
    #[display(fmt = "Rate Limit attempts left: {}", _0)]
    NbAttemptsLeft(usize),
}

pub enum RateLimitReason {
    UserpassIsInvalid,
    UserpassIsNotSet,
}

#[derive(Default)]
pub struct RateLimitContext(AsyncMutex<RateInfosRegistry>);

impl RateLimitContext {
    pub fn from_ctx(ctx: &MmArc) -> Result<Arc<RateLimitContext>, String> {
        Ok(try_s!(from_ctx(&ctx.rate_limit_ctx, move || {
            Ok(RateLimitContext::default())
        })))
    }

    pub async fn is_banned(&self, client_ip: IpAddr) -> bool {
        let rate_infos = self.0.lock().await;
        if let Some(limit) = rate_infos.get(&client_ip) {
            return *limit == LIMIT_FAILED_REQUEST;
        }
        false
    }
}

fn construct_dispatcher_error_from_reason(
    reason: RateLimitReason,
    rate_limit_error: RateLimitError,
) -> MmError<DispatcherError> {
    match reason {
        RateLimitReason::UserpassIsInvalid => MmError::new(DispatcherError::UserpassIsInvalid(rate_limit_error)),
        RateLimitReason::UserpassIsNotSet => MmError::new(DispatcherError::UserpassIsNotSet(rate_limit_error)),
    }
}

pub async fn process_rate_limit(ctx: &MmArc, client: &SocketAddr, reason: RateLimitReason) -> MmError<DispatcherError> {
    let rate_limit_ctx = RateLimitContext::from_ctx(ctx).unwrap();
    let mut rate_limit_registry = rate_limit_ctx.0.lock().await;

    return if let Some(limit) = rate_limit_registry.get_mut(&client.ip()) {
        if *limit == LIMIT_FAILED_REQUEST {
            return construct_dispatcher_error_from_reason(reason, RateLimitError::Banned);
        }
        *limit += 1;
        construct_dispatcher_error_from_reason(reason, RateLimitError::NbAttemptsLeft(LIMIT_FAILED_REQUEST - *limit))
    } else {
        rate_limit_registry.insert(client.ip(), 1);
        construct_dispatcher_error_from_reason(reason, RateLimitError::NbAttemptsLeft(LIMIT_FAILED_REQUEST - 1))
    };
}
