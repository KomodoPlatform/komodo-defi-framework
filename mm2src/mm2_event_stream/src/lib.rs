use serde::Deserialize;
use std::collections::HashMap;
#[cfg(target_arch = "wasm32")] use std::path::PathBuf;

#[cfg(target_arch = "wasm32")]
const DEFAULT_WORKER_PATH: &str = "event_streaming_worker.js";

/// Multi-purpose/generic event type that can easily be used over the event streaming
pub struct Event {
    _type: String,
    message: String,
}

impl Event {
    /// Creates a new `Event` instance with the specified event type and message.
    #[inline]
    pub fn new(event_type: String, message: String) -> Self {
        Self {
            _type: event_type,
            message,
        }
    }

    /// Gets the event type.
    #[inline]
    pub fn event_type(&self) -> &str { &self._type }

    /// Gets the event message.
    #[inline]
    pub fn message(&self) -> &str { &self.message }
}

/// Configuration for event streaming
#[derive(Deserialize)]
pub struct EventStreamConfiguration {
    /// The value to set for the `Access-Control-Allow-Origin` header.
    #[serde(default)]
    pub access_control_allow_origin: String,
    #[serde(default)]
    active_events: HashMap<String, EventConfig>,
    /// The path to the worker script for event streaming.
    #[cfg(target_arch = "wasm32")]
    #[serde(default = "default_worker_path")]
    pub worker_path: PathBuf,
}

#[cfg(target_arch = "wasm32")]
#[inline]
fn default_worker_path() -> PathBuf { PathBuf::from(DEFAULT_WORKER_PATH) }

/// Represents the configuration for a specific event within the event stream.
#[derive(Clone, Default, Deserialize)]
pub struct EventConfig {
    /// The interval in seconds at which the event should be streamed.
    #[serde(default = "default_stream_interval")]
    pub stream_interval_seconds: f64,
}

const fn default_stream_interval() -> f64 { 5. }

impl Default for EventStreamConfiguration {
    fn default() -> Self {
        Self {
            access_control_allow_origin: String::from("*"),
            active_events: Default::default(),
            #[cfg(target_arch = "wasm32")]
            worker_path: default_worker_path(),
        }
    }
}

impl EventStreamConfiguration {
    /// Retrieves the configuration for a specific event by its name.
    #[inline]
    pub fn get_event(&self, event_name: &str) -> Option<EventConfig> { self.active_events.get(event_name).cloned() }

    /// Gets the total number of active events in the configuration.
    #[inline]
    pub fn total_active_events(&self) -> usize { self.active_events.len() }
}

pub mod behaviour;
pub mod controller;
