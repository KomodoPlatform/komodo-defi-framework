//! This module provides a cross-compatible map that associates values with keys and supports expiring entries.
//!
//! Designed for performance-oriented use-cases utilizing `FxHashMap` under the hood,
//! and is not suitable for cryptographic purposes.

use instant::{Duration, Instant};
use rustc_hash::FxHashMap;
use std::hash::Hash;

#[derive(Clone, Debug)]
pub struct ExpirableEntry<V> {
    pub(crate) value: V,
    pub(crate) expires_at: Instant,
}

impl<V> ExpirableEntry<V> {
    pub fn get_element(&self) -> &V { &self.value }

    pub fn update_expiration(&mut self, expires_at: Instant) { self.expires_at = expires_at }
}

impl<K: Eq + Hash, V> Default for ExpirableMap<K, V> {
    fn default() -> Self { Self::new() }
}

/// A map that allows associating values with keys and expiring entries.
/// It is important to note that this implementation does not automatically
/// remove any entries; it is the caller's responsibility to invoke `clear_expired_entries`
/// at specified intervals.
///
/// WARNING: This is designed for performance-oriented use-cases utilizing `FxHashMap`
/// under the hood and is not suitable for cryptographic purposes.
#[derive(Clone, Debug)]
pub struct ExpirableMap<K: Eq + Hash, V>(FxHashMap<K, ExpirableEntry<V>>);

impl<K: Eq + Hash, V> ExpirableMap<K, V> {
    /// Creates a new empty `ExpirableMap`
    #[inline]
    pub fn new() -> Self { Self(FxHashMap::default()) }

    /// Inserts a key-value pair with an expiration duration.
    ///
    /// If a value already exists for the given key, it will be updated and then
    /// the old one will be returned.
    pub fn insert(&mut self, k: K, v: V, exp: Duration) -> Option<V> {
        let entry = ExpirableEntry {
            expires_at: Instant::now() + exp,
            value: v,
        };

        self.0.insert(k, entry).map(|v| v.value)
    }

    /// Removes expired entries from the map.
    pub fn clear_expired_entries(&mut self) { self.0.retain(|_k, v| Instant::now() < v.expires_at); }

    // Removes a key-value pair from the map and returns the associated value if present.
    #[inline]
    pub fn remove(&mut self, k: &K) -> Option<V> { self.0.remove(k).map(|v| v.value) }
}

#[cfg(any(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::cross_test;
    use crate::executor::Timer;

    crate::cfg_wasm32! {
        use wasm_bindgen_test::*;
        wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);
    }

    cross_test!(test_clear_expired_entries, {
        let mut expirable_map = ExpirableMap::new();
        let value = "test_value";
        let exp = Duration::from_secs(1);

        // Insert 2 entries with 1 sec expiration time
        expirable_map.insert("key1".to_string(), value.to_string(), exp);
        expirable_map.insert("key2".to_string(), value.to_string(), exp);

        // Wait for entries to expire
        Timer::sleep(2.).await;

        // Clear expired entries
        expirable_map.clear_expired_entries();

        // We waited for 2 seconds, so we shouldn't have any entry accessible
        assert_eq!(expirable_map.0.len(), 0);

        // Insert 5 entries
        expirable_map.insert("key1".to_string(), value.to_string(), Duration::from_secs(5));
        expirable_map.insert("key2".to_string(), value.to_string(), Duration::from_secs(4));
        expirable_map.insert("key3".to_string(), value.to_string(), Duration::from_secs(7));
        expirable_map.insert("key4".to_string(), value.to_string(), Duration::from_secs(2));
        expirable_map.insert("key5".to_string(), value.to_string(), Duration::from_millis(3750));

        // Wait 2 seconds to expire some entries
        Timer::sleep(2.).await;

        // Clear expired entries
        expirable_map.clear_expired_entries();

        // We waited for 2 seconds, only one entry should expire
        assert_eq!(expirable_map.0.len(), 4);
    });
}
