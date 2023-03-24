#![feature(hash_raw_entry)]
// `mockable` implementation uses these
#![allow(
    clippy::forget_ref,
    clippy::forget_copy,
    clippy::swap_ptr_to_ref,
    clippy::forget_non_drop,
    clippy::let_unit_value
)]

#[macro_use] extern crate common;
#[macro_use] extern crate gstuff;
#[macro_use] extern crate serde_json;
#[macro_use] extern crate serde_derive;
#[macro_use] extern crate ser_error_derive;
#[cfg(test)] extern crate mm2_test_helpers;

pub mod mm2;

#[cfg(all(target_arch = "wasm32", test))] mod wasm_tests;
