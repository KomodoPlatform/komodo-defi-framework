use super::*;
use crate::now_ms;

/// The dummy macro that imitates [`crate::mm_metrics::native::mm_counter`].
/// These macros borrow the `$metrics`, `$name`, `$value` and takes ownership of the `$label_key`, `$label_val` to prevent the `unused_variable` warning.
/// The labels have to be moved because [`metrics_runtime::Sink::increment_counter_with_labels`] also takes ownership of the labels.
#[macro_export]
macro_rules! mm_counter {
    ($metrics:expr, $name:expr, $value:expr) => {{
        let _ = (&$metrics, &$name, &$value); // borrow
    }};
    ($metrics:expr, $name:expr, $value:expr, $($label_key:expr => $label_val:expr),+) => {{
        let _ = (&$metrics, &$name, &$value); // borrow
        let _ = ($($label_key, $label_val),+); // move
    }};
}

/// The dummy macro that imitates [`crate::mm_metrics::native::mm_gauge`].
/// These macros borrow the `$metrics`, `$name`, `$value` and takes ownership of the `$label_key`, `$label_val` to prevent the `unused_variable` warning.
/// The labels have to be moved because [`metrics_runtime::Sink::update_gauge_with_labels`] also takes ownership of the labels.
#[macro_export]
macro_rules! mm_gauge {
    ($metrics:expr, $name:expr, $value:expr) => {{
        let _ = (&$metrics, &$name, &$value); // borrow
    }};
    ($metrics:expr, $name:expr, $value:expr, $($label_key:expr => $label_val:expr),+) => {{
        let _ = (&$metrics, &$name, &$value); // borrow
        let _ = ($($label_key, $label_val),+); // move
    }};
}

/// The dummy macro that imitates [`crate::mm_metrics::native::mm_timing`].
/// These macros borrow the `$metrics`, `$name`, `$start`, `$end` and takes ownership of the `$label_key`, `$label_val` to prevent the `unused_variable` warning.
/// The labels have to be moved because [`metrics_runtime::Sink::record_timing_with_labels`] also takes ownership of the labels.
#[macro_export]
macro_rules! mm_timing {
    ($metrics:expr, $name:expr, $start:expr, $end:expr) => {{
        let _ = (&$metrics, &$name, &$start, &$end); // borrow
    }};
    ($metrics:expr, $name:expr, $start:expr, $end:expr, $($label_key:expr => $label_val:expr),+) => {{
        let _ = (&$metrics, &$name, &$start, &$end); // borrow
        let _ = ($($label_key, $label_val),+); // move
    }};
}

#[derive(Default)]
pub struct Clock {}

impl ClockOps for Clock {
    fn now(&self) -> u64 { now_ms() }
}

#[derive(Default)]
pub struct Metrics {}

impl MetricsOps for Metrics {
    fn init(&self) -> Result<(), String> { Ok(()) }

    fn init_with_dashboard(&self, _log_state: LogWeak, _record_interval: f64) -> Result<(), String> { Ok(()) }

    fn clock(&self) -> Result<Clock, String> { Ok(Clock::default()) }

    fn collect_json(&self) -> Result<Json, String> { Ok(Json::Array(Vec::new())) }
}
