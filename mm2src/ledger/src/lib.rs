mod error;
pub mod transport;

pub use error::{LedgerError, LedgerResult};

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
