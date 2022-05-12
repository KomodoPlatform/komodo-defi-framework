use mm2_ehandle::mm_error::prelude::*;
use futures::channel::{mpsc, oneshot};

#[cfg(target_arch = "wasm32")] pub mod webusb_driver;
#[cfg(target_arch = "wasm32")]
pub use webusb_driver::WebUsbError;

// #[cfg(not(target_arch = "wasm32"))] pub mod hid_driver;
#[cfg(not(target_arch = "wasm32"))] pub mod libusb;
#[cfg(not(target_arch = "wasm32"))] pub use libusb::UsbError;

trait InternalError: Sized {
    fn internal(e: String) -> Self;
}

async fn send_event_recv_response<Event, Ok, Error>(
    event_tx: &mpsc::UnboundedSender<Event>,
    event: Event,
    result_rx: oneshot::Receiver<Result<Ok, MmError<Error>>>,
) -> Result<Ok, MmError<Error>>
where
    Error: InternalError + NotMmError,
{
    if let Err(e) = event_tx.unbounded_send(event) {
        let error = format!("Error sending event: {}", e);
        return MmError::err(Error::internal(error));
    }
    match result_rx.await {
        Ok(result) => result,
        Err(e) => {
            let error = format!("Error receiving result: {}", e);
            MmError::err(Error::internal(error))
        },
    }
}
