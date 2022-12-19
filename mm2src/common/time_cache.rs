// Copyright 2020 Sigma Prime Pty Ltd.
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
// OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

//! This implements a time-based LRU cache for checking gossipsub message duplicates.

use fnv::FnvHashMap;
use std::collections::hash_map::{self,
                                 Entry::{Occupied, Vacant},
                                 Iter, Keys};
use std::collections::VecDeque;
use std::time::Duration;
use wasm_timer::Instant;

#[derive(Debug)]
pub struct ExpiringElement<Element> {
    /// The element that expires
    element: Element,
    /// The expire time.
    expires: Instant,
}

impl<Element> ExpiringElement<Element> {
    pub fn get_element(&self) -> &Element { &self.element }

    pub fn update_expiration(&mut self, expires: Instant) { self.expires = expires }
}

#[derive(Debug)]
pub struct TimeCache<Key, Value> {
    /// Mapping a key to its value together with its latest expire time (can be updated through
    /// reinserts).
    map: FnvHashMap<Key, ExpiringElement<Value>>,
    /// An ordered list of keys by expires time.
    list: VecDeque<ExpiringElement<Key>>,
    /// The time elements remain in the cache.
    ttl: Duration,
}

pub struct OccupiedEntry<'a, K, V> {
    expiration: Instant,
    entry: hash_map::OccupiedEntry<'a, K, ExpiringElement<V>>,
    list: &'a mut VecDeque<ExpiringElement<K>>,
}

impl<'a, K, V> OccupiedEntry<'a, K, V>
where
    K: Eq + std::hash::Hash + Clone,
{
    pub fn into_mut(self) -> &'a mut V { &mut self.entry.into_mut().element }

    #[allow(dead_code)]
    pub fn insert_without_updating_expiration(&mut self, value: V) -> V {
        //keep old expiration, only replace value of element
        ::std::mem::replace(&mut self.entry.get_mut().element, value)
    }

    #[allow(dead_code)]
    pub fn insert_and_update_expiration(&mut self, value: V) -> V {
        //We push back an additional element, the first reference in the list will be ignored
        // since we also updated the expires in the map, see below.
        self.list.push_back(ExpiringElement {
            element: self.entry.key().clone(),
            expires: self.expiration,
        });
        self.entry
            .insert(ExpiringElement {
                element: value,
                expires: self.expiration,
            })
            .element
    }

    pub fn into_mut_with_update_expiration(mut self) -> &'a mut V {
        //We push back an additional element, the first reference in the list will be ignored
        // since we also updated the expires in the map, see below.
        self.list.push_back(ExpiringElement {
            element: self.entry.key().clone(),
            expires: self.expiration,
        });
        self.entry.get_mut().update_expiration(self.expiration);
        &mut self.entry.into_mut().element
    }
}

pub struct VacantEntry<'a, K, V> {
    expiration: Instant,
    entry: hash_map::VacantEntry<'a, K, ExpiringElement<V>>,
    list: &'a mut VecDeque<ExpiringElement<K>>,
}

impl<'a, K, V> VacantEntry<'a, K, V>
where
    K: Eq + std::hash::Hash + Clone,
{
    pub fn insert(self, value: V) -> &'a mut V {
        self.list.push_back(ExpiringElement {
            element: self.entry.key().clone(),
            expires: self.expiration,
        });
        &mut self
            .entry
            .insert(ExpiringElement {
                element: value,
                expires: self.expiration,
            })
            .element
    }
}

pub enum Entry<'a, K: 'a, V: 'a> {
    Occupied(OccupiedEntry<'a, K, V>),
    Vacant(VacantEntry<'a, K, V>),
}

#[allow(dead_code)]
impl<'a, K: 'a, V: 'a> Entry<'a, K, V>
where
    K: Eq + std::hash::Hash + Clone,
{
    pub fn or_insert_with<F: FnOnce() -> V>(self, default: F) -> &'a mut V {
        match self {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(default()),
        }
    }

    pub fn or_insert_with_update_expiration<F: FnOnce() -> V>(self, default: F) -> &'a mut V {
        match self {
            Entry::Occupied(entry) => entry.into_mut_with_update_expiration(),
            Entry::Vacant(entry) => entry.insert(default()),
        }
    }
}

impl<Key, Value> TimeCache<Key, Value>
where
    Key: Eq + std::hash::Hash + Clone,
{
    pub fn new(ttl: Duration) -> Self {
        TimeCache {
            map: FnvHashMap::default(),
            list: VecDeque::new(),
            ttl,
        }
    }

    fn remove_expired_keys(&mut self, now: Instant) {
        while let Some(element) = self.list.pop_front() {
            if element.expires > now {
                self.list.push_front(element);
                break;
            }
            if let Occupied(entry) = self.map.entry(element.element.clone()) {
                if entry.get().expires <= now {
                    entry.remove();
                }
            }
        }
    }

    pub fn entry(&mut self, key: Key) -> Entry<Key, Value> {
        let now = Instant::now();
        self.remove_expired_keys(now);
        match self.map.entry(key) {
            Occupied(entry) => Entry::Occupied(OccupiedEntry {
                expiration: now + self.ttl,
                entry,
                list: &mut self.list,
            }),
            Vacant(entry) => Entry::Vacant(VacantEntry {
                expiration: now + self.ttl,
                entry,
                list: &mut self.list,
            }),
        }
    }

    // Inserts new element and removes any expired elements.
    //
    // If the key was not present this returns `true`. If the value was already present this
    // returns `false`.
    pub fn insert(&mut self, key: Key, value: Value) -> bool {
        if let Entry::Vacant(entry) = self.entry(key) {
            entry.insert(value);
            true
        } else {
            false
        }
    }

    // Removes a certain key even if it didn't expire plus removing other expired keys
    pub fn remove(&mut self, key: Key) -> Option<Value> {
        let result = self.map.remove(&key).map(|el| el.element);
        self.remove_expired_keys(Instant::now());
        result
    }

    /// Empties the entire cache.
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.map.clear();
        self.list.clear();
    }

    pub fn contains_key(&self, key: &Key) -> bool { self.map.contains_key(key) }

    pub fn get(&self, key: &Key) -> Option<&Value> { self.map.get(key).map(|e| &e.element) }

    pub fn len(&self) -> usize { self.map.len() }

    pub fn is_empty(&self) -> bool { self.map.is_empty() }

    pub fn ttl(&self) -> Duration { self.ttl }

    pub fn iter(&self) -> Iter<Key, ExpiringElement<Value>> { self.map.iter() }

    pub fn keys(&self) -> Keys<Key, ExpiringElement<Value>> { self.map.keys() }
}

impl<Key, Value> TimeCache<Key, Value>
where
    Key: Eq + std::hash::Hash + Clone,
    Value: Clone,
{
    pub fn as_hash_map(&self) -> std::collections::HashMap<Key, Value> {
        self.map
            .iter()
            .map(|(key, expiring_el)| (key.clone(), expiring_el.element.clone()))
            .collect()
    }
}

pub struct DuplicateCache<Key: std::hash::Hash>(TimeCache<Key, ()>);

impl<Key> DuplicateCache<Key>
where
    Key: Eq + std::hash::Hash + Clone,
{
    pub fn new(ttl: Duration) -> Self { Self(TimeCache::new(ttl)) }

    // Inserts new elements and removes any expired elements.
    //
    // If the key was not present this returns `true`. If the value was already present this
    // returns `false`.
    pub fn insert(&mut self, key: Key) -> bool {
        if let Entry::Vacant(entry) = self.0.entry(key) {
            entry.insert(());
            true
        } else {
            false
        }
    }

    pub fn contains(&mut self, key: &Key) -> bool { self.0.contains_key(key) }

    // Removes a certain key even if it didn't expire plus removing other expired keys
    #[inline]
    pub fn remove(&mut self, key: Key) { self.0.remove(key); }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn time_cache_added_entries_exist() {
        let mut cache = TimeCache::new(Duration::from_secs(10));

        assert!(cache.insert("t", "tv".to_owned()));
        assert!(cache.insert("e", "ev".to_owned()));

        // Should report that 't' and 't' already exists
        assert!(!cache.insert("t", "td".to_owned()));
        assert!(!cache.insert("e", "ed".to_owned()));

        assert_eq!(cache.get(&"t"), Some(&"tv".to_owned()));
        assert_eq!(cache.get(&"e"), Some(&"ev".to_owned()));
        assert_eq!(cache.get(&"f"), None);
    }

    #[test]
    fn time_cache_expired() {
        let mut cache = TimeCache::new(Duration::from_secs(1));

        assert!(cache.insert("t", "tv".to_owned()));
        assert_eq!(cache.get(&"t"), Some(&"tv".to_owned()));

        std::thread::sleep(Duration::from_millis(500));
        assert!(cache.insert("e", "ev".to_owned()));
        assert_eq!(cache.get(&"t"), Some(&"tv".to_owned()));
        assert_eq!(cache.get(&"e"), Some(&"ev".to_owned()));

        std::thread::sleep(Duration::from_millis(700));
        // insert other value to initiate the expiration
        assert!(cache.insert("f", "fv".to_owned()));
        // must be expired already
        assert_eq!(cache.get(&"t"), None);
        assert_eq!(cache.get(&"e"), Some(&"ev".to_owned()));

        std::thread::sleep(Duration::from_millis(700));
        // insert other value to initiate the expiration
        assert!(cache.insert("d", "dv".to_owned()));
        // must be expired already
        assert_eq!(cache.get(&"t"), None);
        assert_eq!(cache.get(&"e"), None);
    }

    #[test]
    fn cache_added_entries_exist() {
        let mut cache = DuplicateCache::new(Duration::from_secs(10));

        cache.insert("t");
        cache.insert("e");

        // Should report that 't' and 't' already exists
        assert!(!cache.insert("t"));
        assert!(!cache.insert("e"));
    }

    #[test]
    fn cache_entries_expire() {
        let mut cache = DuplicateCache::new(Duration::from_millis(100));

        cache.insert("t");
        assert!(!cache.insert("t"));
        cache.insert("e");
        //assert!(!cache.insert("t"));
        assert!(!cache.insert("e"));
        // sleep until cache expiry
        std::thread::sleep(Duration::from_millis(101));
        // add another element to clear previous cache
        cache.insert("s");

        // should be removed from the cache
        assert!(cache.insert("t"));
    }

    #[test]
    fn test_remove() {
        let mut cache = TimeCache::new(Duration::from_secs(10));

        cache.insert("t", "");
        cache.insert("e", "");
        cache.remove("e");
        assert!(!cache.contains_key(&"e"));
    }
}
