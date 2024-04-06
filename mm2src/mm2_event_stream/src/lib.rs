use serde::Deserialize;
use std::collections::HashMap;
use std::fmt;
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

/// Event types streamed to clients through channels like Server-Sent Events (SSE).
#[derive(Deserialize, Eq, Hash, PartialEq)]
pub enum EventName {
    /// Indicates a change in the balance of a coin.
    CoinBalance,
    /// Event triggered at regular intervals to indicate that the system is operational.
    HEARTBEAT,
    /// Returns p2p network information at a regular interval.
    NETWORK,
}

impl fmt::Display for EventName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CoinBalance => write!(f, "COIN_BALANCE"),
            Self::HEARTBEAT => write!(f, "HEARTBEAT"),
            Self::NETWORK => write!(f, "NETWORK"),
        }
    }
}

/// Error event types used to indicate various kinds of errors to clients through channels like Server-Sent Events (SSE).
pub enum ErrorEventName {
    /// A generic error that doesn't fit any other specific categories.
    GenericError,
    /// Signifies an error related to fetching or calculating the balance of a coin.
    CoinBalanceError,
}

impl fmt::Display for ErrorEventName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GenericError => write!(f, "ERROR"),
            Self::CoinBalanceError => write!(f, "COIN_BALANCE_ERROR"),
        }
    }
}

/// Configuration for event streaming
#[derive(Deserialize)]
pub struct EventStreamConfiguration {
    /// The value to set for the `Access-Control-Allow-Origin` header.
    #[serde(default)]
    pub access_control_allow_origin: String,
    #[serde(default)]
    active_events: HashMap<EventName, EventConfig>,
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
    pub fn get_event(&self, event_name: &EventName) -> Option<EventConfig> {
        self.active_events.get(event_name).cloned()
    }

    /// Gets the total number of active events in the configuration.
    #[inline]
    pub fn total_active_events(&self) -> usize { self.active_events.len() }
}

pub mod behaviour;
pub mod controller;
