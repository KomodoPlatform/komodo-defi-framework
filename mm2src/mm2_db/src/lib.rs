#![allow(uncommon_codepoints)]
#![feature(integer_atomics, panic_info_message)]
#![feature(async_closure)]
#![feature(hash_raw_entry)]
#![feature(negative_impls)]
#![feature(auto_traits)]
#![feature(drain_filter)]

#[cfg(target_arch = "wasm32")]
#[path = "indexed_db/indexed_db.rs"]
pub mod indexed_db;
