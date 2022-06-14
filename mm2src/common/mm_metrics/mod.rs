use crate::log::LogWeak;
pub use metrics_core::labels;
use serde_json::{Value as Json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Weak};

#[cfg(not(target_arch = "wasm32"))] mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::{prometheus, Clock, Metrics, TrySink};

#[cfg(target_arch = "wasm32")] mod wasm;
#[cfg(target_arch = "wasm32")] pub use wasm::{Clock, Metrics};

pub trait MetricsOps {
    /// If the instance was not initialized yet, create the `receiver` else return an error.
    fn init(&self) -> Result<(), String>;

    /// Create new Metrics instance and spawn the metrics recording into the log, else return an error.
    fn init_with_dashboard(&self, log_state: LogWeak, record_interval: f64) -> Result<(), String>;

    /// Handle for sending metric samples.
    fn clock(&self) -> Result<Clock, String>;

    /// Collect the metrics as Json.
    fn collect_json(&self) -> Result<Json, String>;
}

pub trait ClockOps {
    fn now(&self) -> u64;
}

#[derive(Clone, Default)]
pub struct MetricsArc(pub Arc<Metrics>);

impl MetricsOps for MetricsArc {
    fn init(&self) -> Result<(), String> { self.0.init() }

    fn init_with_dashboard(&self, log_state: LogWeak, record_interval: f64) -> Result<(), String> {
        self.0.init_with_dashboard(log_state, record_interval)
    }

    fn clock(&self) -> Result<Clock, String> { self.0.clock() }

    fn collect_json(&self) -> Result<Value, String> { self.0.collect_json() }
}

impl MetricsArc {
    /// Create new `Metrics` instance
    pub fn new() -> MetricsArc { MetricsArc(Arc::new(Default::default())) }

    /// Try to obtain the `Metrics` from the weak pointer.
    pub fn from_weak(weak: &MetricsWeak) -> Option<MetricsArc> { weak.0.upgrade().map(MetricsArc) }

    /// Create a weak pointer from `MetricsWeak`.
    pub fn weak(&self) -> MetricsWeak { MetricsWeak(Arc::downgrade(&self.0)) }
}

#[derive(Clone, Default)]
pub struct MetricsWeak(pub Weak<Metrics>);

impl MetricsWeak {
    /// Create a default MmWeak without allocating any memory.
    pub fn new() -> MetricsWeak { MetricsWeak::default() }

    pub fn dropped(&self) -> bool { self.0.strong_count() == 0 }
}

#[derive(Serialize, Debug, Default, Deserialize)]
pub struct MetricsJson {
    pub metrics: Vec<MetricType>,
}

#[derive(Eq, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
#[serde(tag = "type")]
pub enum MetricType {
    Counter {
        key: String,
        labels: HashMap<String, String>,
        value: u64,
    },
    Gauge {
        key: String,
        labels: HashMap<String, String>,
        value: i64,
    },
    Histogram {
        key: String,
        labels: HashMap<String, String>,
        #[serde(flatten)]
        quantiles: HashMap<String, u64>,
    },
}
