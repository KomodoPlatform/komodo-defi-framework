#![feature(auto_traits, negative_impls)]

pub mod prelude;
pub mod state_machine;
pub mod storable_state_machine;


pub auto trait NotSame {}
impl<X> !NotSame for (X, X) {}
