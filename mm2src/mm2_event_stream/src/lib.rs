use std::collections::HashMap;

use serde::Deserialize;

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
}

/// Represents the configuration for a specific event within the event stream.
#[derive(Clone, Default, Deserialize)]
pub struct EventConfig {
    /// The interval in seconds at which the event should be streamed.
    pub stream_interval_seconds: f64,
}

impl Default for EventStreamConfiguration {
    fn default() -> Self {
        Self {
            access_control_allow_origin: String::from("*"),
            active_events: Default::default(),
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
