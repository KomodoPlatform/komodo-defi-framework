#![allow(uncommon_codepoints)]
#![feature(async_closure)]
#![feature(drain_filter)]
#![recursion_limit = "512"]
#![feature(test)]
#![feature(hash_raw_entry)]
#![feature(map_first_last)]

#[macro_use] extern crate common;
#[macro_use] extern crate gstuff;
#[macro_use] extern crate serde_json;
#[macro_use] extern crate serde_derive;
#[macro_use] extern crate serialization_derive;
#[macro_use] extern crate ser_error_derive;

#[path = "mm2.rs"] mod mm2;

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
#[global_allocator]
static GLOBAL: jemallocator::Jemalloc = jemallocator::Jemalloc;

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        mm2::mm2_main()
    }
}
