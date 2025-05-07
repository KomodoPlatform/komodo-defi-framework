#![feature(negative_impls)]
#![allow(clippy::doc_lazy_continuation)]

#[cfg(target_arch = "wasm32")]
#[path = "indexed_db/indexed_db.rs"]
pub mod indexed_db;
