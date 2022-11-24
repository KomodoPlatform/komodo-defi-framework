#![feature(hash_raw_entry)]

#[macro_use] extern crate common;
#[macro_use] extern crate gstuff;
#[macro_use] extern crate serde_json;
#[macro_use] extern crate serde_derive;
#[macro_use] extern crate ser_error_derive;
#[cfg(test)] extern crate mm2_test_helpers;

pub mod mm2;

#[cfg(all(target_arch = "wasm32", test))] mod wasm_tests;
