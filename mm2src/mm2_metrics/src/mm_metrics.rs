use common::executor::{spawn, Timer};
#[cfg(not(target_arch = "wasm32"))] use common::log::error;
use common::log::{LogArc, LogWeak};
use itertools::Itertools;
use metrics::{Key, Label};
use mm2_err_handle::prelude::*;
use serde_json::Value;
use std::sync::{atomic::Ordering, Arc};
use std::{collections::HashMap, slice::Iter};

use crate::MmMetricsError;
use crate::{common::log::Tag, MetricsOps, MmMetricsResult, MmRecorder};

type MetricLabels = Vec<Label>;

pub(crate) type MetricNameValueMap = HashMap<String, PreparedMetric>;

/// Construct Vec<Label> from a slice of strings.
#[macro_export]
macro_rules! mm_label {
    ($($label_key:expr => $label_val:expr),+) => {{
         vec![$(($label_key.to_owned(), $label_val.to_owned())),+]
    }};
}

/// Increment counter if an MmArc is not dropped yet and metrics system is initialized already.
#[macro_export]
macro_rules! mm_counter {
    ($metrics:expr, $name:expr, $value:expr) => {{
        use $crate::metrics::Recorder;
        if let Some(recorder) = $crate::recorder::TryRecorder::try_recorder(&$metrics) {
            let key = $crate::metrics::Key::from_static_name($name);
            let counter = recorder.register_counter(&key);
            counter.increment($value);
        };
    }};

    // Register and increment counter with label.
    ($metrics:expr, $name:expr, $value:expr, $($label_key:expr => $label_val:expr),+) => {{
        use $crate::metrics::Recorder;
        if let Some(recorder) = $crate::recorder::TryRecorder::try_recorder(&$metrics) {
            let key = $crate::metrics::Key::from_parts($name, mm_label!($($label_key => $label_val),+).as_slice());
            let counter = recorder.register_counter(&key);
            counter.increment($value);
        };
    }};
}

/// Update gauge if an MmArc is not dropped yet and metrics system is initialized already.
#[macro_export]
macro_rules! mm_gauge {
    ($metrics:expr, $name:expr, $value:expr) => {{
        use $crate::metrics::Recorder;
        if let Some(recorder) = $crate::recorder::TryRecorder::try_recorder(&$metrics){
            let key = $crate::metrics::Key::from_static_name($name);
            let gauge = recorder.register_gauge(&key);
            gauge.set($value);
        }
    }};

    // Register and set gauge with label.
    ($metrics:expr, $name:expr, $value:expr, $($label_key:expr => $label_val:expr),+) => {{
        use $crate::metrics::Recorder;
        if let Some(recorder) = $crate::recorder::TryRecorder::try_recorder(&$metrics){
            let key = $crate::metrics::Key::from_parts($name, mm_label!($($label_key => $label_val),+).as_slice());
            let gauge = recorder.register_gauge(&key);
            gauge.set($value);
        }
    }};
}

/// Update gauge if an MmArc is not dropped yet and metrics system is initialized already.
#[macro_export]
macro_rules! mm_timing {
    ($metrics:expr, $name:expr, $value:expr) => {{
        use $crate::metrics::Recorder;
        if let Some(recorder) = $crate::recorder::TryRecorder::try_recorder(&$metrics){
            let key =$crate::metrics::Key::from_static_name($name);
            let histo = recorder.register_histogram(&key);
            histo.record($value);
        }
    }};

    // Register and record histogram with label.
    ($metrics:expr, $name:expr, $value:expr, $($label_key:expr => $label_val:expr),+) => {{
        use $crate::metrics::Recorder;
        if let Some(recorder) = $crate::recorder::TryRecorder::try_recorder(&$metrics){
            let key = $crate::metrics::Key::from_parts($name, mm_label!($($label_key => $label_val),+).as_slice());
            let histo = recorder.register_histogram(&key);
            histo.record($value);
        }
    }};
}

/// Market Maker Metrics, used as inner to get metrics data and exporting.
#[derive(Default, Clone)]
pub struct Metrics {
    pub(crate) recorder: Arc<MmRecorder>,
}

impl Metrics {
    /// Collect the metrics in Prometheus format.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn collect_prometheus_format(&self) -> String { self.recorder.render_prometheus() }
}

impl MetricsOps for Metrics {
    fn init(&self) { Metrics::default(); }

    fn init_with_dashboard(&self, log_state: LogWeak, interval: f64) -> MmMetricsResult<()> {
        let recorder = self.recorder.clone();
        let runner = TagObserver::log_tag_metrics(log_state, recorder, interval);
        spawn(runner);
        Ok(())
    }

    /// Collect prepared metrics json from the recorder.
    fn collect_json(&self) -> MmMetricsResult<Value> {
        serde_json::to_value(self.recorder.prepare_json()).map_to_mm(|err| MmMetricsError::Internal(err.to_string()))
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct TagMetric {
    pub tags: Vec<Tag>,
    pub message: String,
}

#[derive(PartialEq, PartialOrd)]
pub(crate) enum PreparedMetric {
    Unsigned(u64),
    Float(f64),
    Histogram(MmHistogram),
}

pub struct TagObserver;

impl TagObserver {
    /// This is for collecting and logging `prepare_tag_metrics` to dashboard.
    ///
    /// Used with `init_with_dashboard`.
    pub async fn log_tag_metrics(log_state: LogWeak, recorder: Arc<MmRecorder>, interval: f64) {
        loop {
            Timer::sleep(interval).await;
            let log_state = match LogArc::from_weak(&log_state) {
                Some(log_arc) => log_arc,
                _ => return,
            };

            log!(">>>>>>>>>> DEX metrics <<<<<<<<<");

            Self::prepare_tag_metrics(&recorder)
                .into_iter()
                .for_each(|(labels, name_value_map)| {
                    let tags = labels_to_tags(labels.iter());
                    let message = name_value_map_to_message(&name_value_map);
                    log_state.log_deref_tags("", tags, &message);
                });
        }
    }

    /// Prepare tag metrics for logging in `log_tag_metrics`.
    fn prepare_tag_metrics(recorder: &MmRecorder) -> HashMap<MetricLabels, MetricNameValueMap> {
        let mut output = HashMap::new();

        for (key, counter) in recorder.registry.get_counter_handles() {
            let value = counter.get_inner().load(Ordering::Acquire);
            map_metrics_to_prepare_tag_metric_output(key, PreparedMetric::Unsigned(value), &mut output);
        }

        for (key, gauge) in recorder.registry.get_gauge_handles() {
            let value = f64::from_bits(gauge.get_inner().load(Ordering::Acquire));
            map_metrics_to_prepare_tag_metric_output(key, PreparedMetric::Float(value), &mut output);
        }

        for (key, histo) in recorder.registry.get_histogram_handles() {
            if let Some(values) = MmHistogram::new(&histo.get_inner().data()) {
                map_metrics_to_prepare_tag_metric_output(key, PreparedMetric::Histogram(values), &mut output);
            }
        }
        output
    }
}

/// Used for parsing metrics to `prepare_tag_metric_output`
fn map_metrics_to_prepare_tag_metric_output(
    key: Key,
    value: PreparedMetric,
    output: &mut HashMap<MetricLabels, MetricNameValueMap>,
) {
    let (metric_name, labels) = key.into_parts();
    output
        .entry(labels)
        .or_insert_with(MetricNameValueMap::new)
        .insert(metric_name.as_str().to_string(), value);
}

pub(crate) fn labels_to_tags(labels: Iter<Label>) -> Vec<Tag> {
    labels
        .map(|label| Tag {
            key: label.key().to_string(),
            val: Some(label.value().to_string()),
        })
        .collect()
}

/// Used for parsing `MetricNameValueMap` into Message(loggable string).
pub(crate) fn name_value_map_to_message(name_value_map: &MetricNameValueMap) -> String {
    name_value_map
        .iter()
        .sorted_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Less))
        .map(|(key, value)| match value {
            crate::mm_metrics::PreparedMetric::Unsigned(v) => format!("{}={:?}", key, v),
            crate::mm_metrics::PreparedMetric::Float(v) => format!("{}={:?}", key, v),
            crate::mm_metrics::PreparedMetric::Histogram(v) => format!("{}={:?}", key, v.to_tag_message()),
        })
        .join(" ")
}

#[derive(PartialEq, PartialOrd)]
pub(crate) struct MmHistogram {
    count: usize,
    min: f64,
    max: f64,
}

impl MmHistogram {
    /// Create new MmHistogram from `&[f64]`.
    ///
    /// Return None if data.len() <= 0.
    pub(crate) fn new(data: &[f64]) -> Option<MmHistogram> {
        let (min, max) = data.iter().minmax().into_option()?;
        Some(MmHistogram {
            count: data.len(),
            min: *min,
            max: *max,
        })
    }

    /// Create new MmHistogram from `&[f64]`.
    pub(crate) fn to_tag_message(&self) -> String { format!("count={} min={} max={}", self.count, self.min, self.max) }

    /// Create new MmHistogram from `&[f64]`.
    pub(crate) fn to_json_quantiles(&self) -> HashMap<String, f64> {
        let mut result = HashMap::new();
        result.insert("count".to_owned(), self.count as f64);
        result.insert("min".to_owned(), self.min);
        result.insert("max".to_owned(), self.max);

        result
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod prometheus {
    use crate::{MetricsArc, MetricsWeak};

    use super::*;
    use futures::future::{Future, FutureExt};
    use hyper::http::{self, header, Request, Response, StatusCode};
    use hyper::service::{make_service_fn, service_fn};
    use hyper::{Body, Server};
    use mm2_err_handle::prelude::MmError;
    use std::convert::Infallible;
    use std::net::SocketAddr;

    #[derive(Clone)]
    pub struct PrometheusCredentials {
        pub userpass: String,
    }

    pub fn spawn_prometheus_exporter(
        metrics: MetricsWeak,
        address: SocketAddr,
        shutdown_detector: impl Future<Output = ()> + 'static + Send,
        credentials: Option<PrometheusCredentials>,
    ) -> Result<(), String> {
        let make_svc = make_service_fn(move |_conn| {
            let metrics = metrics.clone();
            let credentials = credentials.clone();
            futures::future::ready(Ok::<_, Infallible>(service_fn(move |req| {
                futures::future::ready(scrape_handle(req, metrics.clone(), credentials.clone()))
            })))
        });

        let server = Server::try_bind(&address)
            .map_err(|e| e.to_string())?
            .http1_half_close(false) // https://github.com/hyperium/hyper/issues/1764
            .serve(make_svc)
            .with_graceful_shutdown(shutdown_detector);

        let server = server.then(|r| {
            if let Err(err) = r {
                error!("{}", err);
            };
            futures::future::ready(())
        });

        spawn(server);
        Ok(())
    }

    fn scrape_handle(
        req: Request<Body>,
        metrics: MetricsWeak,
        credentials: Option<PrometheusCredentials>,
    ) -> Result<Response<Body>, http::Error> {
        fn on_error(status: StatusCode, error: MmError<MmMetricsError>) -> Result<Response<Body>, http::Error> {
            error!("{}", error.to_string());
            Response::builder().status(status).body(Body::empty()).map_err(|err| {
                error!("{}", err);
                err
            })
        }

        if req.uri() != "/metrics" {
            return on_error(
                StatusCode::BAD_REQUEST,
                MmError::new(MmMetricsError::Internal(format!(
                    "Warning Prometheus: unexpected URI {}",
                    req.uri()
                ))),
            );
        }

        if let Some(credentials) = credentials {
            if let Err(err) = check_auth_credentials(&req, credentials) {
                return on_error(StatusCode::UNAUTHORIZED, err);
            }
        }

        let metrics = match MetricsArc::from_weak(&metrics) {
            Some(m) => m,
            _ => {
                return on_error(
                    StatusCode::BAD_REQUEST,
                    MmError::new(MmMetricsError::Internal(
                        "Warning Prometheus: metrics system unavailable".to_string(),
                    )),
                )
            },
        };

        let body = Body::from(metrics.0.collect_prometheus_format());

        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(body)
            .map_err(|err| {
                error!("{}", err);
                err
            })
    }

    fn check_auth_credentials(req: &Request<Body>, expected: PrometheusCredentials) -> MmMetricsResult<()> {
        let header_value = req
            .headers()
            .get(header::AUTHORIZATION)
            .ok_or_else(|| {
                MmMetricsError::PrometheusTransport("Warning Prometheus: authorization required".to_string())
            })
            .and_then(|header| {
                Ok(header
                    .to_str()
                    .map_err(|e| MmMetricsError::PrometheusTransport(e.to_string())))?
            })?;

        let expected = format!("Basic {}", base64::encode_config(&expected.userpass, base64::URL_SAFE));

        if header_value != expected {
            return Err(MmError::new(MmMetricsError::Internal(format!(
                "Warning Prometheus: invalid credentials: {}",
                header_value
            ))));
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {

    use std::time::Duration;

    use crate::{MetricsArc, MetricsOps};

    use common::{block_on,
                 executor::Timer,
                 log::{LogArc, LogState}};
    use quanta::Clock;

    #[test]
    fn test_collect_json() {
        let metrics = MetricsArc::new();

        metrics.init();

        mm_counter!(metrics, "rpc.traffic.tx", 62, "coin" => "BTC");
        mm_counter!(metrics, "rpc.traffic.rx", 105, "coin" => "BTC");

        mm_counter!(metrics, "rpc.traffic.tx", 30, "coin" => "BTC");
        mm_counter!(metrics, "rpc.traffic.rx", 44, "coin" => "BTC");

        mm_counter!(metrics, "rpc.traffic.tx", 54, "coin" => "KMD");
        mm_counter!(metrics, "rpc.traffic.rx", 158, "coin" => "KMD");

        mm_gauge!(metrics, "rpc.connection.count", 3.0, "coin" => "KMD");
        mm_gauge!(metrics, "rpc.connection.count", 5.0, "coin" => "KMD");

        let delta = 34382022725155.0 - 34381019796149.0;
        mm_timing!(metrics,
                   "rpc.query.spent_time",
                   // ~ 1 second
                   delta, // delta
                   "coin" => "KMD",
                   "method" => "blockchain.transaction.get");

        let delta = 34384023173373.0 - 34382022774105.0;
        mm_timing!(metrics,
                   "rpc.query.spent_time",
                   // ~ 2 second
                    delta, // delta
                   "coin" => "KMD",
                   "method" => "blockchain.transaction.get");

        let expected = serde_json::json!({
            "metrics": [
                {
                    "key": "rpc.traffic.rx",
                    "labels": { "coin": "BTC" },
                    "type": "counter",
                    "value": 149
                },
                {
                    "key": "rpc.traffic.tx",
                    "labels": { "coin": "KMD" },
                    "type": "counter",
                    "value": 54
                },
                {
                    "key": "rpc.traffic.tx",
                    "labels": { "coin": "BTC" },
                    "type": "counter",
                    "value": 92
                },
                {
                    "key": "rpc.traffic.rx",
                    "labels": { "coin": "KMD" },
                    "type": "counter",
                    "value": 158
                },
                {
                    "count": 2.0,
                    "key": "rpc.query.spent_time",
                    "labels": { "coin": "KMD", "method": "blockchain.transaction.get" },
                    "max": 2000399268.0,
                    "min": 1002929006.0,
                    "type": "histogram"
                },
                {
                    "key": "rpc.connection.count",
                    "labels": { "coin": "KMD" },
                    "type": "gauge",
                    "value": 5.0
                }
            ]
        });

        let mut actual = metrics.collect_json().unwrap();
        let actual = actual["metrics"].as_array_mut().unwrap();
        for expected in expected["metrics"].as_array().unwrap() {
            let index = actual.iter().position(|metric| metric == expected).expect(&format!(
                "Couldn't find expected metric: {:#?} \n in {:#?}",
                expected, actual
            ));
            actual.remove(index);
        }

        assert!(
            actual.is_empty(),
            "More metrics collected than expected. Excess metrics: {:?}",
            actual
        );
    }

    #[test]
    fn test_dashboard() {
        let log_state = LogArc::new(LogState::in_memory());
        let mm_metrics = MetricsArc::new();

        mm_metrics.init_with_dashboard(log_state.weak(), 6.).unwrap();

        let clock = Clock::new();
        let last: quanta::Instant = clock.now();

        mm_counter!(mm_metrics, "rpc.traffic.tx", 62, "coin" => "BTC");
        mm_counter!(mm_metrics, "rpc.traffic.rx", 105, "coin"=> "BTC");

        mm_counter!(mm_metrics, "rpc.traffic.tx", 54, "coin" => "KMD");
        mm_counter!(mm_metrics, "rpc.traffic.rx", 158, "coin" => "KMD");

        mm_gauge!(mm_metrics, "rpc.connection.count", 3.0, "coin" => "KMD");

        block_on(async { Timer::sleep(6.).await });

        let delta: Duration = clock.now() - last;
        mm_timing!(mm_metrics,
                    "rpc.query.spent_time",
                    delta,
                    "coin" => "KMD",
                    "method" => "blockchain.transaction.get");

        block_on(async { Timer::sleep(6.).await });

        mm_counter!(mm_metrics, "rpc.traffic.tx", 30, "coin" => "BTC");
        mm_counter!(mm_metrics, "rpc.traffic.rx", 44, "coin" => "BTC");

        mm_gauge!(mm_metrics, "rpc.connection.count", 5.0, "coin" => "KMD");

        let delta: Duration = clock.now() - last;
        mm_timing!(mm_metrics,
                    "rpc.query.spent_time",
                    delta,
                    "coin" => "KMD",
                    "method" => "blockchain.transaction.get");

        // measure without labels
        mm_counter!(mm_metrics, "test.counter", 0);
        mm_gauge!(mm_metrics, "test.gauge", 1.0);
        let delta: Duration = clock.now() - last;
        mm_timing!(mm_metrics, "test.uptime", delta);

        block_on(async { Timer::sleep(6.).await });
    }

    #[test]
    fn test_prometheus_format() {
        let mm_metrics = MetricsArc::new();

        mm_metrics.init();

        mm_counter!(mm_metrics, "rpc.traffic.tx", 54, "coin" => "KMD");
        mm_counter!(mm_metrics, "rpc.traffic.rx", 158, "coin" => "KMD");

        mm_gauge!(mm_metrics, "rpc.connection.count", 3.0, "coin" => "KMD");
        mm_gauge!(mm_metrics, "rpc.connection.count", 5.0, "coin" => "KMD");

        mm_timing!(mm_metrics,
                    "rpc.query.spent_time",
                    4.5,
                    "coin"=> "KMD",
                    "method"=>"blockchain.transaction.get");

        let expected = concat!(
            "# TYPE rpc_traffic_tx counter\n",
            "rpc_traffic_tx{coin=\"KMD\"} 54\n\n",
            "# TYPE rpc_traffic_rx counter\n",
            "rpc_traffic_rx{coin=\"KMD\"} 158\n\n",
            "# TYPE rpc_connection_count gauge\n",
            "rpc_connection_count{coin=\"KMD\"} 5\n\n",
            "# TYPE rpc_query_spent_time histogram\n",
            "rpc_query_spent_time_sum 4.5\n",
            "rpc_query_spent_time_count 1\n\n",
        );
        let actual = mm_metrics.0.collect_prometheus_format();
        assert_eq!(actual, expected);
    }
}
