/// The timeout for the electrum server to respond to a request.
pub const ELECTRUM_REQUEST_TIMEOUT: f64 = 20.;
/// The default (can be overridden) maximum timeout to establish a connection with the electrum server.
/// This included connecting to the server and querying the server version.
pub const DEFAULT_CONNECTION_ESTABLISHMENT_TIMEOUT: f64 = 20.;
/// Wait this long before pinging again.
pub const PING_INTERVAL: f64 = 30.;
/// Used to cutoff the server connection after not receiving any response for that long.
/// This only makes sense if we have sent a request to the server. So we need to keep `PING_INTERVAL`
/// lower than this value, otherwise we might disconnect servers that are perfectly responsive but just
/// haven't received any requests from us for a while.
pub const CUTOFF_TIMEOUT: f64 = 60.;
/// Initial server suspension time.
pub const FIRST_SUSPEND_TIME: u64 = 10;
/// The timeout used by the background task of the connection manager to re-check the manager's health.
pub const BACKGROUND_TASK_WAIT_TIMEOUT: f64 = (5 * 60) as f64;
/// Electrum methods that should not be sent without forcing the connection to be established first.
pub const NO_FORCE_CONNECT_METHODS: &[&str] = &[
    // The server should already be connected if we are querying for its version, don't force connect.
    "server.version",
];
/// Electrum methods that should be sent to all connections even after receiving a response from a subset of them.
/// Note that this is only applicable to active/maintained connections. If an electrum request fails by all maintained
/// connections, a fallback using all connections will *NOT* be attempted.
pub const SEND_TO_ALL_METHODS: &[&str] = &[
    // A ping should be sent to all connections even if we got a response from one of them early.
    "server.ping",
];
/// Electrum RPC method for headers subscription.
pub const BLOCKCHAIN_HEADERS_SUB_ID: &str = "blockchain.headers.subscribe";
/// Electrum RPC method for script/address subscription.
pub const BLOCKCHAIN_SCRIPTHASH_SUB_ID: &str = "blockchain.scripthash.subscribe";
