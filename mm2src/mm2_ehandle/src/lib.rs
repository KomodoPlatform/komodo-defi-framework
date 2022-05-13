#![allow(uncommon_codepoints)]
#![feature(integer_atomics, panic_info_message)]
#![feature(async_closure)]
#![feature(hash_raw_entry)]
#![feature(negative_impls)]
#![feature(auto_traits)]
#![feature(drain_filter)]

pub mod map_mm_error;
pub mod map_to_mm;
pub mod map_to_mm_fut;
pub mod mm_error;
pub mod mm_json_error;
pub mod or_mm_error;
