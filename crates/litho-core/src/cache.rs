//! Кэш тайлов (план 6.6, задача T14).
//!
//! Шарды на `Mutex`, ограниченный LRU по времени обращения. Кэш хранит только
//! результаты чистых функций: попал запрос в кэш или нет — ответ одинаковый.
//! Вычисление идёт **вне блокировки**, иначе вложенный запрос к другому шарду
//! мог бы встать в тупик.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Mutex;

/// Ёмкость кэша не влияет на значения полей, поэтому это инфраструктурная
/// константа, а не число генератора в `Params` (9.5).
pub const CACHE_CAPACITY: usize = 512;

// Slot indices form a doubly linked LRU list. Nothing is allocated on a hit
// and a cold insertion into a full shard reuses its oldest slot. This matters
// for W: querying shore cells for a plate touches many more keys than fit.
const END: usize = usize::MAX;
#[derive(Debug)]
struct Slot<K, V> {
    key: K,
    value: V,
    older: usize,
    newer: usize,
}
#[derive(Debug)]
struct Shard<K, V> {
    index: HashMap<K, usize>,
    slots: Vec<Slot<K, V>>,
    oldest: usize,
    newest: usize,
}
impl<K: Eq + Hash + Clone, V: Clone> Shard<K, V> {
    fn new() -> Self {
        Self {
            index: HashMap::new(),
            slots: vec![],
            oldest: END,
            newest: END,
        }
    }
    fn promote(&mut self, i: usize) {
        if self.newest == i {
            return;
        }
        let (older, newer) = (self.slots[i].older, self.slots[i].newer);
        if older == END {
            self.oldest = newer;
        } else {
            self.slots[older].newer = newer;
        }
        self.slots[newer].older = older;
        self.slots[self.newest].newer = i;
        self.slots[i].older = self.newest;
        self.slots[i].newer = END;
        self.newest = i;
    }
    fn get(&mut self, key: &K) -> Option<V> {
        let i = *self.index.get(key)?;
        self.promote(i);
        Some(self.slots[i].value.clone())
    }
    fn insert(&mut self, key: K, value: V, cap: usize) {
        let i = if self.slots.len() == cap {
            let i = self.oldest;
            self.index.remove(&self.slots[i].key);
            self.promote(i);
            self.slots[i].key = key.clone();
            self.slots[i].value = value;
            i
        } else {
            let i = self.slots.len();
            self.slots.push(Slot {
                key: key.clone(),
                value,
                older: self.newest,
                newer: END,
            });
            if self.newest == END {
                self.oldest = i;
            } else {
                self.slots[self.newest].newer = i;
            }
            self.newest = i;
            i
        };
        self.index.insert(key, i);
    }
}

#[derive(Debug)]
pub struct ShardedCache<K, V> {
    shards: Vec<Mutex<Shard<K, V>>>,
    capacity: usize,
}

impl<K, V> ShardedCache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    pub fn new(shards: usize, capacity: usize) -> Self {
        let shards = shards.max(1);
        let capacity = capacity.max(shards);
        let mut list = Vec::with_capacity(shards);
        for _ in 0..shards {
            list.push(Mutex::new(Shard::new()));
        }
        Self {
            shards: list,
            capacity,
        }
    }

    fn shard_index(&self, key: &K) -> usize {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() as usize) % self.shards.len()
    }

    /// Чтение с обновлением LRU; промах не меняет содержимое кэша.
    pub fn get(&self, key: K) -> Option<V> {
        let index = self.shard_index(&key);
        let mut shard = self.shards[index].lock().expect("кэш не отравлен");
        shard.get(&key)
    }

    /// Значение для ключа: из кэша или вычисленное один раз.
    pub fn get_or_insert<F>(&self, key: K, compute: F) -> V
    where
        F: FnOnce() -> V,
    {
        let index = self.shard_index(&key);
        if let Some(value) = {
            let mut shard = self.shards[index].lock().expect("кэш не отравлен");
            shard.get(&key)
        } {
            return value;
        }
        let value = compute();
        let mut shard = self.shards[index].lock().expect("кэш не отравлен");
        // A concurrent miss may already have published the canonical slot.
        // Never replace it: OnceLock users must converge on the same builder.
        if let Some(found) = shard.get(&key) {
            return found;
        }
        shard.insert(key, value.clone(), self.capacity / self.shards.len());
        value
    }

    /// Для диагностики: сколько тайлов сейчас в кэше.
    pub fn len(&self) -> usize {
        self.shards
            .iter()
            .map(|shard| shard.lock().expect("кэш не отравлен").slots.len())
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::{CACHE_CAPACITY, ShardedCache};

    #[test]
    fn cold_warm_and_tiny_cache_give_the_same_answer() {
        let warm = ShardedCache::<i64, i64>::new(8, CACHE_CAPACITY);
        let tiny = ShardedCache::<i64, i64>::new(1, 1);
        let cold = ShardedCache::<i64, i64>::new(4, CACHE_CAPACITY);
        let mut from_warm = Vec::new();
        let mut from_tiny = Vec::new();
        let mut from_cold = Vec::new();
        for k in 0..200_i64 {
            from_warm.push(warm.get_or_insert(k % 7, || (k % 7) * (k % 7) + 1));
            from_tiny.push(tiny.get_or_insert(k % 7, || (k % 7) * (k % 7) + 1));
            from_cold.push(cold.get_or_insert(k % 7, || (k % 7) * (k % 7) + 1));
        }
        assert_eq!(from_warm, from_tiny);
        assert_eq!(from_warm, from_cold);
        // Кэш размера 1 действительно выбрасывает старые тайлы.
        assert!(tiny.len() <= 1);
    }

    #[test]
    fn lru_reuses_exact_oldest_slot_after_reads_and_racing_inserts() {
        use std::sync::{Arc, Barrier};
        let c = ShardedCache::new(1, 3);
        for n in 1..=3 {
            c.get_or_insert(n, || n);
        }
        assert_eq!(c.get(1), Some(1));
        c.get_or_insert(4, || 4);
        assert_eq!(c.get(2), None);
        assert_eq!(c.get(3), Some(3));
        c.get_or_insert(5, || 5);
        assert_eq!(c.get(1), None);
        let gate = Barrier::new(2);
        let c = ShardedCache::new(1, 1);
        std::thread::scope(|s| {
            let insert = || {
                c.get_or_insert(1, || {
                    gate.wait();
                    Arc::new(0)
                })
            };
            let a = s.spawn(insert);
            let b = s.spawn(insert);
            assert!(Arc::ptr_eq(&a.join().unwrap(), &b.join().unwrap()));
        });
    }
    #[test]
    fn compute_runs_only_once_per_key_while_cached() {
        let cache = ShardedCache::<i64, i64>::new(2, 100);
        let first = cache.get_or_insert(11, || 123);
        let second = cache.get_or_insert(11, || 999);
        assert_eq!((first, second), (123, 123));
    }
}
