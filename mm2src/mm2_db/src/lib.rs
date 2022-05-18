#![feature(negative_impls)]
#![feature(auto_traits)]

#[cfg(target_arch = "wasm32")]
#[path = "indexed_db/indexed_db.rs"]
pub mod indexed_db;
