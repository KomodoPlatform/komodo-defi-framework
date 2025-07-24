use serde::Deserialize;

#[derive(Deserialize)]
#[serde(default)]
/// The network-related configuration of the event streaming interface.
// TODO: This better fits in mm2_net but then we would have circular dependency error trying to import it in mm2_core.
pub struct EventStreamingConfiguration {
    pub worker_path: String,
    pub access_control_allow_origin: String,
}

impl Default for EventStreamingConfiguration {
    fn default() -> Self {
        Self {
            worker_path: "event_streaming_worker.js".to_string(),
            access_control_allow_origin: "*".to_string(),
        }
    }
}
