//! Кэш тайлов (план 6.6, задача T14).
//!
//! Шарды на `Mutex`, ограниченный LRU по времени обращения. Кэш хранит только
//! результаты чистых функций: попал запрос в кэш или нет — ответ одинаковый.
//! Вычисление идёт **вне блокировки**, иначе вложенный запрос к другому шарду
//! мог бы встать в тупик.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Ёмкость кэша не влияет на значения полей, поэтому это инфраструктурная
/// константа, а не число генератора в `Params` (9.5).
pub const CACHE_CAPACITY: usize = 512;

#[derive(Debug)]
pub struct ShardedCache<K, V> {
    shards: Vec<Mutex<HashMap<K, (V, u64)>>>,
    capacity: usize,
    clock: AtomicU64,
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
            list.push(Mutex::new(HashMap::new()));
        }
        Self {
            shards: list,
            capacity,
            clock: AtomicU64::new(0),
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
        shard.get_mut(&key).map(|(value, stamp)| {
            *stamp = self.clock.fetch_add(1, Ordering::Relaxed);
            value.clone()
        })
    }

    /// Значение для ключа: из кэша или вычисленное один раз.
    pub fn get_or_insert<F>(&self, key: K, compute: F) -> V
    where
        F: FnOnce() -> V,
    {
        let index = self.shard_index(&key);
        if let Some(value) = {
            let mut shard = self.shards[index].lock().expect("кэш не отравлен");
            shard.get_mut(&key).map(|(value, stamp)| {
                *stamp = self.clock.fetch_add(1, Ordering::Relaxed);
                value.clone()
            })
        } {
            return value;
        }
        let value = compute();
        let stamp = self.clock.fetch_add(1, Ordering::Relaxed);
        let mut shard = self.shards[index].lock().expect("кэш не отравлен");
        let per_shard = (self.capacity / self.shards.len()).max(1);
        if shard.len() >= per_shard {
            if let Some(oldest) = shard
                .iter()
                .min_by_key(|(_, (_, time))| *time)
                .map(|(key, _)| key.clone())
            {
                shard.remove(&oldest);
            }
        }
        shard.insert(key, (value.clone(), stamp));
        value
    }

    /// Для диагностики: сколько тайлов сейчас в кэше.
    pub fn len(&self) -> usize {
        self.shards
            .iter()
            .map(|shard| shard.lock().expect("кэш не отравлен").len())
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
    fn compute_runs_only_once_per_key_while_cached() {
        let cache = ShardedCache::<i64, i64>::new(2, 100);
        let first = cache.get_or_insert(11, || 123);
        let second = cache.get_or_insert(11, || 999);
        assert_eq!((first, second), (123, 123));
    }
}
