use std::time::Duration;

pub trait WithInternal {
    fn internal(desc: String) -> Self;
}

pub trait WithTimeout {
    fn timeout(duration: Duration) -> Self;
}
