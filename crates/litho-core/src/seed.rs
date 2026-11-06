//! Соли и подсиды (план 6.3, 7.1, модуль `seed.rs` в 9.3).
//!
//! Список солей — одна таблица констант. Менять соль в пределах версии
//! генератора нельзя: это поменяет весь мир.

use crate::hash::{fnv1a64, hash3, splitmix64, unit};

// Полный список солей v0. Новые соли добавляются в конец с комментарием,
// в каком слое они используются.
pub const SALT_PLATES_SEED: &str = "plates.seed";
pub const SALT_PLATES_ID: &str = "plates.id";
pub const SALT_PLATES_PROPERTIES: &str = "plates.properties";
pub const SALT_PLATES_VELOCITY: &str = "plates.velocity";
pub const SALT_WARP_PLATES_X: &str = "warp.plates.x";
pub const SALT_WARP_PLATES_Z: &str = "warp.plates.z";
pub const SALT_POLES_SEED: &str = "poles.seed";
pub const SALT_POLES_WARP_X: &str = "warp.poles.x";
pub const SALT_POLES_WARP_Z: &str = "warp.poles.z";
pub const SALT_NOISE_DETAIL: &str = "noise.detail";
pub const SALT_CRUST_KAPPA: &str = "crust.kappa";
pub const SALT_CRUST_DETAIL: &str = "crust.detail";
pub const SALT_HOTSPOT: &str = "crust.hotspot";
pub const SALT_BOUNDARY_SIDE: &str = "boundary.side";
pub const SALT_SPAWN: &str = "spawn";
pub const SALT_PHI_NOISE: &str = "poles.phi_noise";
pub const SALT_CLASS_JITTER: &str = "classify.jitter";
pub const SALT_VANILLA_E: &str = "vanilla.erosion";
pub const SALT_VANILLA_W: &str = "vanilla.weirdness";
pub const SALT_VANILLA_S: &str = "vanilla.sign";
pub const SALT_RIVERS: &str = "rivers.source";
pub const SALT_RIVERS_MEANDER: &str = "rivers.meander";

/// Сид Minecraft знаковый 64-битный; биты переносятся без изменения.
pub const fn from_world_seed(seed: i64) -> u64 {
    seed as u64
}

/// Подсид назначения: `splitmix64(seed ^ fnv1a64(соль))`.
pub fn subseed(seed: u64, salt: &str) -> u64 {
    splitmix64(seed ^ fnv1a64(salt))
}

/// Хэш ячейки с уже выделенным подсидом.
pub fn cell_hash(subseed_value: u64, i: i64, j: i64, k: i64) -> u64 {
    hash3(subseed_value, i, j, k)
}

/// Последовательность случайных чисел одной ячейки: `h0 = hash3(...)`,
/// дальше `h1 = splitmix64(h0)`, `h2 = splitmix64(h1)` и т. д.
/// Порядок вызовов фиксируется рядом с местом использования.
pub struct CellRng {
    h: u64,
}

impl CellRng {
    pub fn from_hash(h: u64) -> Self {
        Self { h }
    }

    pub fn next_u64(&mut self) -> u64 {
        let value = self.h;
        self.h = splitmix64(self.h);
        value
    }

    pub fn next_unit(&mut self) -> f64 {
        unit(self.next_u64())
    }

    /// Равномерно в `[lo, hi)`.
    pub fn next_range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_unit()
    }
}

impl Default for CellRng {
    fn default() -> Self {
        Self::from_hash(0)
    }
}

#[cfg(test)]
mod tests {
    use super::{CellRng, SALT_PLATES_SEED, SALT_PLATES_VELOCITY, from_world_seed, subseed};
    use crate::hash::hash3;

    #[test]
    fn negative_world_seed_keeps_bits() {
        assert_eq!(from_world_seed(-1), u64::MAX);
        assert_eq!(from_world_seed(0), 0);
    }

    #[test]
    fn different_salts_give_different_subseeds() {
        assert_ne!(
            subseed(42, SALT_PLATES_SEED),
            subseed(42, SALT_PLATES_VELOCITY)
        );
        assert_eq!(subseed(42, SALT_PLATES_SEED), subseed(42, SALT_PLATES_SEED));
    }

    #[test]
    fn cell_rng_follows_splitmix_chain() {
        let h0 = hash3(subseed(42, SALT_PLATES_SEED), 3, -5, 0);
        let mut rng = CellRng::from_hash(h0);
        assert_eq!(rng.next_u64(), h0);
        assert_eq!(rng.next_u64(), crate::hash::splitmix64(h0));
    }
}
