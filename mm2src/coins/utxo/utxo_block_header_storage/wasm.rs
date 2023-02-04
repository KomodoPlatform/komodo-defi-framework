mod block_header_table;
mod indexeddb_block_header_storage;

#[cfg(target_arch = "wasm32")]
pub use block_header_table::{BlockHeaderStorageTable, HEIGHT_TICKER_INDEX};
#[cfg(target_arch = "wasm32")]
pub use indexeddb_block_header_storage::IDBBlockHeadersStorage;
