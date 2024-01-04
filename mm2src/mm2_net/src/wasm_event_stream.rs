use mm2_core::mm_ctx::MmArc;
use serde_json::json;

/// Handles broadcasted messages from `mm2_event_stream` continuously for WASM.
pub async fn handle_worker_stream(ctx: MmArc) {
    let config = ctx
        .event_stream_configuration
        .as_ref()
        .expect("Event stream configuration couldn't be found. This should never happen.");

    let mut channel_controller = ctx.stream_channel_controller.clone();
    let mut rx = channel_controller.create_channel(config.total_active_events());

    while let Some(event) = rx.recv().await {
        let data = json!({
            "_type": event.event_type(),
            "message": event.message(),
        });

        let worker = web_sys::Worker::new("worker.js").expect("Missing worker.js");
        let message_js = wasm_bindgen::JsValue::from_str(&data.to_string());

        worker.post_message(&message_js)
            .expect("Incompatible browser!\nSee https://developer.mozilla.org/en-US/docs/Web/API/Worker/postMessage#browser_compatibility for details.");
    }
}
