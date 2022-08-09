#[cfg(not(target_arch = "wasm32"))] pub mod sql_constraint;
#[cfg(not(target_arch = "wasm32"))] pub mod sql_create;
#[cfg(not(target_arch = "wasm32"))] pub mod sql_insert;
#[cfg(not(target_arch = "wasm32"))] pub mod sql_query;
#[cfg(not(target_arch = "wasm32"))] pub mod sql_value;
#[cfg(not(target_arch = "wasm32"))] pub mod sqlite;
