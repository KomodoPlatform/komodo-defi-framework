/// The timeout for the electrum server to respond to a request.
pub const ELECTRUM_REQUEST_TIMEOUT: f64 = 60.;
/// The default (can be overridden) maximum timeout to establish a connection with the electrum server.
/// This included connecting to the server and querying the server version.
pub const DEFAULT_CONNECTION_ESTABLISHMENT_TIMEOUT: f64 = 60.;
/// Wait this long before pinging again.
pub const PING_INTERVAL: f64 = 30.;
/// Used to cutoff the server connection after not receiving any response for that long.
/// This only makes sense if we have sent a request to the server. So we need to keep `PING_INTERVAL`
/// lower than this value, otherwise we might disconnect servers that are perfectly responsive but just
/// haven't received any requests from us for a while.
pub const CUTOFF_TIMEOUT: f64 = 60.;
/// Initial server suspension time.
pub const FIRST_SUSPEND_TIME: u64 = 10;

// Some electrum RPC methods.
pub const BLOCKCHAIN_HEADERS_SUB_ID: &str = "blockchain.headers.subscribe";
pub const BLOCKCHAIN_SCRIPTHASH_SUB_ID: &str = "blockchain.scripthash.subscribe";
