#![feature(negative_impls)]

pub mod prelude;
pub mod state_machine;
pub mod storable_state_machine;


pub trait NotSame {}
impl<X> !NotSame for (X, X) {}
