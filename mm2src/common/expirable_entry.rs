//! This module provides a cross-compatible map that associates values with keys and supports expiring entries.
//!
//! Designed for performance-oriented use-cases utilizing `FxHashMap` under the hood,
//! and is not suitable for cryptographic purposes.

use instant::{Duration, Instant};
#[derive(Clone, Debug)]
pub struct ExpirableEntry<V> {
    pub(crate) value: V,
    pub(crate) expires_at: Instant,
}

impl<V> ExpirableEntry<V> {
    #[inline(always)]
    pub fn new(v: V, exp: Duration) -> Self {
        Self {
            expires_at: Instant::now() + exp,
            value: v,
        }
    }

    #[inline(always)]
    pub fn get_element(&self) -> &V { &self.value }

    #[inline(always)]
    pub fn update_value(&mut self, v: V) { self.value = v }

    #[inline(always)]
    pub fn update_expiration(&mut self, expires_at: Instant) { self.expires_at = expires_at }

    /// Checks whether entry has longer ttl than the given one.
    #[inline(always)]
    pub fn has_longer_life_than(&self, min_ttl: Duration) -> bool { self.expires_at > Instant::now() + min_ttl }
}
