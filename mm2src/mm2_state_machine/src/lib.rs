#![feature(allocator_api, auto_traits, negative_impls)]

pub mod prelude;
pub mod state_machine;
pub mod storable_state_machine;

use std::alloc::Allocator;

pub auto trait NotSame {}
impl<X> !NotSame for (X, X) {}
// Makes the error conversion work for structs/enums containing Box<dyn ...>
impl<T: ?Sized, A: Allocator> NotSame for Box<T, A> {}
