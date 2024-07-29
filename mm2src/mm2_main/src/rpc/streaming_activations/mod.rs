mod balance;
mod disable;
mod fee_estimation;
mod heartbeat;
mod network;
mod orders;
mod swaps;

// Re-exports
pub use balance::*;
pub use disable::*;
pub use fee_estimation::*;
pub use heartbeat::*;
pub use network::*;
pub use orders::*;
pub use swaps::*;

#[derive(Serialize)]
/// The success/ok response for any event streaming activation request.
/// Note that we don't have a unified request. It is rather defined per event streaming activation.
pub struct EnableStreamingResponse {
    pub streamer_id: String,
    // FIXME: Consider returning the applied config here (might be different from the one the client requested).
    // pub config: Json,
}

impl EnableStreamingResponse {
    fn new(streamer_id: String) -> Self { Self { streamer_id } }
}