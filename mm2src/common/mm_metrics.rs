use crate::executor::{spawn, Timer};
use crate::log::{LogArc, LogWeak, Tag};
use gstuff::Constructible;
use hdrhistogram::Histogram;
use itertools::Itertools;
use metrics_core::{Key, Label, Drain, Observe, Observer, ScopedString, Builder};
use metrics_runtime::{Receiver, observers::PrometheusBuilder};
pub use metrics_runtime::Sink;
use metrics_util::{parse_quantiles, Quantile};
use serde_json::{self as json, Value as Json};
use std::collections::HashMap;
use std::fmt::Write as WriteFmt;
use std::ops::Deref;
use std::slice::Iter;
use std::sync::{Arc, Weak};

/// Increment counter if an MmArc is not dropped yet and metrics system is initialized already.
#[macro_export]
macro_rules! mm_counter {
    ($metrics:expr, $name:expr, $value:expr) => {{
        if let Some(mut sink) = $crate::mm_metrics::TrySink::try_sink(&$metrics) {
            sink.increment_counter($name, $value);
        }
    }};

    ($metrics:expr, $name:expr, $value:expr, $($labels:tt)*) => {{
        use metrics::labels;
        if let Some(mut sink) = $crate::mm_metrics::TrySink::try_sink(&$metrics) {
            let labels = labels!( $($labels)* );
            sink.increment_counter_with_labels($name, $value, labels);
        }
    }};
}

/// Update gauge if an MmArc is not dropped yet and metrics system is initialized already.
#[macro_export]
macro_rules! mm_gauge {
    ($metrics:expr, $name:expr, $value:expr) => {{
        if let Some(mut sink) = $crate::mm_metrics::TrySink::try_sink(&$metrics) {
            sink.update_gauge($name, $value);
        }
    }};

    ($metrics:expr, $name:expr, $value:expr, $($labels:tt)*) => {{
        use metrics::labels;
        if let Some(mut sink) = $crate::mm_metrics::TrySink::try_sink(&$metrics) {
            let labels = labels!( $($labels)* );
            sink.update_gauge_with_labels($name, $value, labels);
        }
    }};
}

/// Pass new timing value if an MmArc is not dropped yet and metrics system is initialized already.
#[macro_export]
macro_rules! mm_timing {
    ($metrics:expr, $name:expr, $start:expr, $end:expr) => {{
        if let Some(mut sink) = $crate::mm_metrics::TrySink::try_sink(&$metrics) {
            sink.record_timing($name, $start, $end);
        }
    }};

    ($metrics:expr, $name:expr, $start:expr, $end:expr, $($labels:tt)*) => {{
        use metrics::labels;
        if let Some(mut sink) = $crate::mm_metrics::TrySink::try_sink(&$metrics) {
            let labels = labels!( $($labels)* );
            sink.record_timing_with_labels($name, $start, $end, labels);
        }
    }};
}

#[cfg(feature = "native")]
pub mod prometheus {
    use crate::wio::CORE;
    use futures01::{self, future, Future};
    use futures::compat::Future01CompatExt;
    use futures::future::FutureExt;
    use hyper::http::{self, header, Request, Response, StatusCode};
    use hyper::{Body, Server};
    use hyper::service::{make_service_fn, service_fn};
    use std::convert::Infallible;
    use std::net::SocketAddr;
    use super::*;

    #[derive(Clone)]
    pub struct PrometheusCredentials {
        pub userpass: String,
    }

    pub fn spawn_prometheus_exporter(metrics: MetricsWeak,
                                     address: SocketAddr,
                                     shutdown_detector: impl Future<Item=(), Error=()> + 'static + Send,
                                     credentials: Option<PrometheusCredentials>)
                                     -> Result<(), String> {
        let make_svc = make_service_fn(move |_conn| {
            let metrics = metrics.clone();
            let credentials = credentials.clone();
            Ok::<_, Infallible>(
                service_fn(move |req| {
                    future::result(scrape_handle(req, metrics.clone(), credentials.clone()))
                })
            )
        });

        let server = try_s!(Server::try_bind(&address))
            .http1_half_close(false) // https://github.com/hyperium/hyper/issues/1764
            .executor(try_s!(CORE.lock()).executor())
            .serve(make_svc)
            .with_graceful_shutdown(shutdown_detector);

        let server = server.then(|r| -> Result<_, ()> {
            if let Err(err) = r {
                log!((err));
            };
            Ok(())
        });

        spawn(server.compat().map(|_| ()));
        Ok(())
    }

    fn scrape_handle(req: Request<Body>,
                     metrics: MetricsWeak,
                     credentials: Option<PrometheusCredentials>)
                     -> Result<Response<Body>, http::Error> {
        fn on_error(status: StatusCode, error: String) -> Result<Response<Body>, http::Error> {
            log!((error));
            Response::builder()
                .status(status)
                .body(Body::empty())
                .map_err(|err| {
                    log!((err));
                    err
                })
        }

        if req.uri() != "/metrics" {
            return on_error(StatusCode::BAD_REQUEST, ERRL!("Warning Prometheus: unexpected URI {}", req.uri()));
        }

        if let Some(credentials) = credentials {
            if let Err(err) = check_auth_credentials(&req, credentials) {
                return on_error(StatusCode::UNAUTHORIZED, err);
            }
        }

        let metrics = match MetricsArc::from_weak(&metrics) {
            Some(m) => m,
            _ => return on_error(StatusCode::BAD_REQUEST, ERRL!("Warning Prometheus: metrics system unavailable")),
        };

        let body = match metrics.collect_prometheus_format() {
            Ok(body) => Body::from(body),
            _ => return on_error(StatusCode::BAD_REQUEST, ERRL!("Warning Prometheus: metrics system is not initialized yet")),
        };

        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(body)
            .map_err(|err| {
                log!((err));
                err
            })
    }

    fn check_auth_credentials(req: &Request<Body>, expected: PrometheusCredentials) -> Result<(), String> {
        let header_value = req.headers()
            .get(header::AUTHORIZATION)
            .ok_or(ERRL!("Warning Prometheus: authorization required"))
            .and_then(|header| Ok(try_s!(header.to_str())))
            ?;

        let expected = format!("Basic {}", base64::encode_config(&expected.userpass, base64::URL_SAFE));

        if header_value != expected {
            return Err(format!("Warning Prometheus: invalid credentials: {}", header_value));
        }

        Ok(())
    }
}

pub trait TrySink {
    fn try_sink(&self) -> Option<Sink>;
}

/// Default quantiles are "min" and "max"
const QUANTILES: &[f64] = &[0.0, 1.0];

#[derive(Default)]
pub struct Metrics {
    /// `Receiver` receives and collect all the metrics sent through the `sink`.
    /// The `receiver` can be initialized only once time.
    receiver: Constructible<Receiver>,
}

impl Metrics {
    /// If the instance was not initialized yet, create the `receiver` else return an error.
    pub fn init(&self) -> Result<(), String> {
        if self.receiver.is_some() {
            return ERR!("metrics system is initialized already");
        }

        let receiver = try_s!(Receiver::builder().build());
        let _ = try_s!(self.receiver.pin(receiver));

        Ok(())
    }

    /// Create new Metrics instance and spawn the metrics recording into the log, else return an error.
    pub fn init_with_dashboard(&self, log_state: LogWeak, record_interval: f64) -> Result<(), String> {
        self.init()?;

        let controller = self.receiver.as_option().unwrap().controller();

        let observer = TagObserver::new(QUANTILES);
        let exporter = TagExporter { log_state, controller, observer };

        spawn(exporter.run(record_interval));

        Ok(())
    }

    /// Handle for sending metric samples.
    pub fn sink(&self) -> Result<Sink, String> {
        Ok(try_s!(self.try_receiver()).sink())
    }

    /// Collect the metrics as Json.
    pub fn collect_json(&self) -> Result<Json, String> {
        let receiver = try_s!(self.try_receiver());
        let controller = receiver.controller();

        let mut observer = JsonObserver::new(QUANTILES);

        controller.observe(&mut observer);

        observer.into_json()
    }

    /// Collect the metrics in Prometheus format.
    pub fn collect_prometheus_format(&self) -> Result<String, String> {
        let receiver = try_s!(self.try_receiver());
        let controller = receiver.controller();

        let mut observer = PrometheusBuilder::new().set_quantiles(QUANTILES).build();
        controller.observe(&mut observer);

        Ok(observer.drain())
    }

    /// Try get receiver.
    fn try_receiver(&self) -> Result<&Receiver, String> {
        self.receiver.ok_or("metrics system is not initialized yet".into())
    }
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

#[derive(Clone)]
pub struct MetricsArc(pub Arc<Metrics>);

impl Deref for MetricsArc {
    type Target = Metrics;
    fn deref(&self) -> &Metrics {
        &*self.0
    }
}

impl TrySink for MetricsArc {
    fn try_sink(&self) -> Option<Sink> {
        self.sink().ok()
    }
}

impl MetricsArc {
    /// Create new `Metrics` instance
    pub fn new() -> MetricsArc {
        MetricsArc(Arc::new(Default::default()))
    }

    /// Try to obtain the `Metrics` from the weak pointer.
    pub fn from_weak(weak: &MetricsWeak) -> Option<MetricsArc> {
        weak.0.upgrade().map(|arc| MetricsArc(arc))
    }

    /// Create a weak pointer from `MetricsWeak`.
    pub fn weak(&self) -> MetricsWeak {
        MetricsWeak(Arc::downgrade(&self.0))
    }
}

#[derive(Clone)]
pub struct MetricsWeak(pub Weak<Metrics>);

impl TrySink for MetricsWeak {
    fn try_sink(&self) -> Option<Sink> {
        let metrics = MetricsArc::from_weak(&self)?;
        metrics.sink().ok()
    }
}

impl MetricsWeak {
    /// Create a default MmWeak without allocating any memory.
    pub fn new() -> MetricsWeak {
        MetricsWeak(Default::default())
    }

    pub fn dropped(&self) -> bool {
        self.0.strong_count() == 0
    }
}

type MetricName = ScopedString;

type MetricLabels = Vec<Label>;

type MetricNameValueMap = HashMap<MetricName, Integer>;

#[derive(Clone, Eq, PartialEq, PartialOrd, Ord)]
enum Integer {
    Signed(i64),
    Unsigned(u64),
}

impl ToString for Integer {
    fn to_string(&self) -> String {
        match self {
            Integer::Signed(x) => format!("{}", x),
            Integer::Unsigned(x) => format!("{}", x),
        }
    }
}

struct PreparedMetric {
    tags: Vec<Tag>,
    message: String,
}

/// Observes metrics and histograms in Tag format.
struct TagObserver {
    /// Supported quantiles like Min, 0.5, 0.8, Max
    quantiles: Vec<Quantile>,
    /// Metric:Value pair matching an unique set of labels.
    metrics: HashMap<MetricLabels, MetricNameValueMap>,
    /// Histograms present set of time measurements and analysis over the measurements
    histograms: HashMap<Key, Histogram<u64>>,
}

impl TagObserver {
    fn new(quantiles: &[f64]) -> Self {
        TagObserver {
            quantiles: parse_quantiles(quantiles),
            metrics: Default::default(),
            histograms: Default::default(),
        }
    }

    fn prepare_metrics(&self) -> Vec<PreparedMetric> {
        self.metrics.iter()
            .map(|(labels, name_value_map)| {
                let tags = labels_to_tags(labels.iter());
                let message = name_value_map_to_message(name_value_map);

                PreparedMetric { tags, message }
            })
            .collect()
    }

    fn prepare_histograms(&self) -> Vec<PreparedMetric> {
        self.histograms.iter()
            .map(|(key, hist)| {
                let tags = labels_to_tags(key.labels());
                let message = format!("{}: {}", key.name(), hist_to_message(hist, &self.quantiles));

                PreparedMetric { tags, message }
            })
            .collect()
    }

    fn insert_metric(&mut self, key: Key, value: Integer) {
        let (name, labels) = key.into_parts();
        self.metrics.entry(labels)
            .and_modify(|name_value_map| {
                name_value_map.insert(name.clone(), value.clone());
            })
            .or_insert({
                let mut name_value_map = HashMap::new();
                name_value_map.insert(name, value);
                name_value_map
            });
    }

    /// Clear metrics or histograms if it's necessary
    /// after an exporter has turned the observer's metrics and histograms.
    fn on_turned(&mut self) {
        // clear histograms because they can be duplicated
        self.histograms.clear();
        // don't clear metrics because the keys don't changes often
    }
}

impl Observer for TagObserver {
    fn observe_counter(&mut self, key: Key, value: u64) {
        self.insert_metric(key, Integer::Unsigned(value))
    }

    fn observe_gauge(&mut self, key: Key, value: i64) {
        self.insert_metric(key, Integer::Signed(value))
    }

    fn observe_histogram(&mut self, key: Key, values: &[u64]) {
        let entry = self.histograms
            .entry(key)
            .or_insert({
                // Use default significant figures value.
                // For more info on `sigfig` see the Historgam::new_with_bounds().
                let sigfig = 3;
                match Histogram::new(sigfig) {
                    Ok(x) => x,
                    Err(err) => {
                        log!("failed to create histogram: "(err));
                        // do nothing on error
                        return;
                    }
                }
            });

        for value in values {
            if let Err(err) = entry.record(*value) {
                log!("failed to observe histogram value: "(err));
            }
        }
    }
}

/// Observes metrics and histograms in Tag format.
struct JsonObserver {
    /// Supported quantiles like Min, 0.5, 0.8, Max.
    quantiles: Vec<Quantile>,
    /// Collected metrics and histograms as serializable and deserializable structure.
    metrics: MetricsJson,
}

impl Observer for JsonObserver {
    fn observe_counter(&mut self, key: Key, value: u64) {
        let (key, labels) = key.into_parts();

        let metric = MetricType::Counter {
            key: key.to_string(),
            labels: labels_into_parts(labels.iter()),
            value,
        };

        self.metrics.metrics.push(metric);
    }

    fn observe_gauge(&mut self, key: Key, value: i64) {
        let (key, labels) = key.into_parts();

        let metric = MetricType::Gauge {
            key: key.to_string(),
            labels: labels_into_parts(labels.iter()),
            value,
        };

        self.metrics.metrics.push(metric);
    }

    fn observe_histogram(&mut self, key: Key, values: &[u64]) {
        let (key, labels) = key.into_parts();

        // Use default significant figures value.
        // For more info on `sigfig` see the Historgam::new_with_bounds().
        let sigfig = 3;
        let mut histogram = match Histogram::new(sigfig) {
            Ok(x) => x,
            Err(err) => {
                log!("failed to create histogram: "(err));
                // do nothing on error
                return;
            }
        };

        for value in values {
            if let Err(err) = histogram.record(*value) {
                log!("failed to observe histogram value: "(err));
            }
        }

        let count = histogram.len() as u64;
        let mut quantiles = hist_at_quantiles(histogram, &self.quantiles);
        // add total quantiles number
        quantiles.insert("count".into(), count);

        let metric = MetricType::Histogram {
            key: key.to_string(),
            labels: labels_into_parts(labels.iter()),
            quantiles,
        };

        self.metrics.metrics.push(metric);
    }
}

impl JsonObserver {
    fn new(quantiles: &[f64]) -> Self {
        JsonObserver {
            quantiles: parse_quantiles(quantiles),
            metrics: Default::default(),
        }
    }

    fn into_json(self) -> Result<Json, String> {
        json::to_value(self.metrics).map_err(|err| ERRL!("{}", err))
    }
}

/// Exports metrics by converting them to a Tag format and log them using log::Status.
struct TagExporter<C>
{
    /// Using a weak reference by default in order to avoid circular references and leaks.
    log_state: LogWeak,
    /// Handle for acquiring metric snapshots.
    controller: C,
    /// Handle for converting snapshots into log.
    observer: TagObserver,
}

impl<C> TagExporter<C>
    where C: Observe {
    /// Run endless async loop
    async fn run(mut self, interval: f64) {
        loop {
            Timer::sleep(interval).await;
            self.turn();
        }
    }

    /// Observe metrics and histograms and record it into the log in Tag format
    fn turn(&mut self) {
        let log_state = match LogArc::from_weak(&self.log_state) {
            Some(x) => x,
            // MmCtx is dropped already
            _ => return
        };

        log!(">>>>>>>>>> DEX metrics <<<<<<<<<");

        // Observe means fill the observer's metrics and histograms with actual values
        self.controller.observe(&mut self.observer);

        for PreparedMetric { tags, message } in self.observer.prepare_metrics() {
            log_state.log_deref_tags("", tags, &message);
        }

        for PreparedMetric { tags, message } in self.observer.prepare_histograms() {
            log_state.log_deref_tags("", tags, &message);
        }

        self.observer.on_turned();
    }
}

fn labels_to_tags(labels: Iter<Label>) -> Vec<Tag> {
    labels
        .map(|label| Tag {
            key: label.key().to_string(),
            val: Some(label.value().to_string()),
        })
        .collect()
}

fn labels_into_parts(labels: Iter<Label>) -> HashMap<String, String> {
    labels
        .map(|label| (label.key().to_string(), label.value().to_string()))
        .collect()
}

fn name_value_map_to_message(name_value_map: &MetricNameValueMap) -> String {
    let mut message = String::with_capacity(256);
    match wite!(message, for (key, value) in name_value_map.iter().sorted() { (key) "=" (value.to_string()) } separated {' '}) {
        Ok(_) => message,
        Err(err) => {
            log!("Error " (err) " on format hist to message");
            String::new()
        }
    }
}

fn hist_at_quantiles(hist: Histogram<u64>, quantiles: &[Quantile]) -> HashMap<String, u64> {
    quantiles
        .into_iter()
        .map(|quantile| {
            let key = quantile.label().to_string();
            let val = hist.value_at_quantile(quantile.value());
            (key, val)
        })
        .collect()
}

fn hist_to_message(hist: &Histogram<u64>, quantiles: &[Quantile]) -> String {
    let mut message = String::with_capacity(256);
    let fmt_quantiles = quantiles
        .iter()
        .map(|quantile| {
            let key = quantile.label().to_string();
            let val = hist.value_at_quantile(quantile.value());
            format!("{}={}", key, val)
        });

    match wite!(message,
                "count=" (hist.len())
                if quantiles.is_empty() { "" } else { " " }
                for q in fmt_quantiles { (q) } separated {' '}
    ) {
        Ok(_) => message,
        Err(err) => {
            log!("Error " (err) " on format hist to message");
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{block_on, log::LogArc};
    use crate::log::LogState;
    use super::*;

    #[test]
    fn test_initialization() {
        let log_state = LogArc::new(LogState::in_memory());
        let metrics = MetricsArc::new();

        // metrics system is not initialized yet
        assert!(metrics.sink().is_err());

        unwrap!(metrics.init());
        assert!(metrics.init().is_err());
        assert!(metrics.init_with_dashboard(log_state.weak(), 1.).is_err());

        let _ = unwrap!(metrics.sink());
    }

    #[test]
    #[ignore]
    fn test_dashboard() {
        let log_state = LogArc::new(LogState::in_memory());
        let metrics = MetricsArc::new();

        metrics.init_with_dashboard(log_state.weak(), 5.).unwrap();
        let sink = metrics.sink().unwrap();

        let start = sink.now();

        mm_counter!(metrics, "rpc.traffic.tx", 62, "coin" => "BTC");
        mm_counter!(metrics, "rpc.traffic.rx", 105, "coin"=> "BTC");

        mm_counter!(metrics, "rpc.traffic.tx", 54, "coin" => "KMD");
        mm_counter!(metrics, "rpc.traffic.rx", 158, "coin" => "KMD");

        mm_gauge!(metrics, "rpc.connection.count", 3, "coin" => "KMD");

        let end = sink.now();
        mm_timing!(metrics,
                   "rpc.query.spent_time",
                   start,
                   end,
                   "coin" => "KMD",
                   "method" => "blockchain.transaction.get");

        block_on(async { Timer::sleep(6.).await });

        mm_counter!(metrics, "rpc.traffic.tx", 30, "coin" => "BTC");
        mm_counter!(metrics, "rpc.traffic.rx", 44, "coin" => "BTC");

        mm_gauge!(metrics, "rpc.connection.count", 5, "coin" => "KMD");

        let end = sink.now();
        mm_timing!(metrics,
                   "rpc.query.spent_time",
                   start,
                   end,
                   "coin"=> "KMD",
                   "method"=>"blockchain.transaction.get");

        // measure without labels
        mm_counter!(metrics, "test.counter", 0);
        mm_gauge!(metrics, "test.gauge", 1);
        let end = sink.now();
        mm_timing!(metrics, "test.uptime", start, end);

        block_on(async { Timer::sleep(6.).await });
    }

    #[test]
    fn test_collect_json() {
        let metrics = MetricsArc::new();

        metrics.init().unwrap();
        let mut sink = metrics.sink().unwrap();

        mm_counter!(metrics, "rpc.traffic.tx", 62, "coin" => "BTC");
        mm_counter!(metrics, "rpc.traffic.rx", 105, "coin" => "BTC");

        mm_counter!(metrics, "rpc.traffic.tx", 30, "coin" => "BTC");
        mm_counter!(metrics, "rpc.traffic.rx", 44, "coin" => "BTC");

        mm_counter!(metrics, "rpc.traffic.tx", 54, "coin" => "KMD");
        mm_counter!(metrics, "rpc.traffic.rx", 158, "coin" => "KMD");

        mm_gauge!(metrics, "rpc.connection.count", 3, "coin" => "KMD");

        // counter, gauge and timing may be collected also by sink API
        sink.update_gauge_with_labels("rpc.connection.count", 5, &[("coin", "KMD")]);

        mm_timing!(metrics,
                   "rpc.query.spent_time",
                   // ~ 1 second
                   34381019796149, // start
                   34382022725155, // end
                   "coin" => "KMD",
                   "method" => "blockchain.transaction.get");

        mm_timing!(metrics,
                   "rpc.query.spent_time",
                   // ~ 2 second
                   34382022774105, // start
                   34384023173373, // end
                   "coin" => "KMD",
                   "method" => "blockchain.transaction.get");

        let expected = json!({
            "metrics": [
                {
                    "key": "rpc.traffic.tx",
			        "labels": { "coin": "BTC" },
			        "type": "counter",
			        "value": 92
                },
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
                    "key": "rpc.traffic.rx",
			        "labels": { "coin": "KMD" },
			        "type": "counter",
			        "value": 158
                },
                {
                    "count": 2,
                    "key": "rpc.query.spent_time",
			        "labels": { "coin": "KMD", "method": "blockchain.transaction.get" },
			        "max": 2000683007,
			        "min": 1002438656,
			        "type": "histogram"
                },
                {
                    "key": "rpc.connection.count",
			        "labels": { "coin": "KMD" },
			        "type": "gauge",
			        "value": 5
                }
            ]
        });

        let mut actual = metrics.collect_json().unwrap();

        let actual = actual["metrics"].as_array_mut().unwrap();
        for expected in expected["metrics"].as_array().unwrap() {
            let index = actual.iter()
                .position(|metric| metric == expected)
                .expect(&format!("Couldn't find expected metric: {:?}", expected));
            actual.remove(index);
        }

        assert!(actual.is_empty(), "More metrics collected than expected. Excess metrics: {:?}", actual);
    }
}
