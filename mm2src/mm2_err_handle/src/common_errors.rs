use std::time::Duration;

/// The trait is implemented for those error enumerations that have `Internal` variant.
pub trait WithInternal {
    fn internal(desc: String) -> Self;
}

/// The trait is implemented for those error enumerations that have `Timeout` variant.
pub trait WithTimeout {
    fn timeout(duration: Duration) -> Self;
}
