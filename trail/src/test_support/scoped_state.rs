use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Mutex;

pub(crate) struct ScopedTestState<K: Eq + Hash + Clone + 'static, V: 'static> {
    key: K,
    map: &'static Mutex<HashMap<K, V>>,
}

impl<K: Eq + Hash + Clone + 'static, V: 'static> ScopedTestState<K, V> {
    pub(crate) fn install(map: &'static Mutex<HashMap<K, V>>, key: K, value: V) -> Self {
        let previous = map
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key.clone(), value);
        assert!(previous.is_none(), "scoped test key was already installed");
        Self { key, map }
    }
}

impl<K: Eq + Hash + Clone + 'static, V: 'static> Drop for ScopedTestState<K, V> {
    fn drop(&mut self) {
        self.map
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.key);
    }
}
