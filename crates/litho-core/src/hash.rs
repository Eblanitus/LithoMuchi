//! Хэши и случайные числа ячеек (план 7.1).
//!
//! Никакого состояния: одинаковый вход всегда даёт одинаковый выход, поэтому
//! порядок вычислений и число потоков не влияют на результат.

pub const FNV_OFFSET: u64 = 0xCBF2_9CE4_8422_2325;
pub const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

const K1: u64 = 0x9E37_79B9_7F4A_7C15;
const K2: u64 = 0xC2B2_AE3D_27D4_EB4F;
const K3: u64 = 0x1656_67B1_9E37_79F9;

/// `2^-53`: ровно представимая степень двойки, без вызова `powf`.
const TWO_POW_NEG_53: f64 = 1.0 / 9_007_199_254_740_992.0;

/// FNV-1a для строковых солей. Соль задана строкой, чтобы её нельзя было
/// случайно поменять, переставив числовые константы в коде.
pub fn fnv1a64(text: &str) -> u64 {
    let mut h = FNV_OFFSET;
    for byte in text.as_bytes() {
        h = (h ^ u64::from(*byte)).wrapping_mul(FNV_PRIME);
    }
    h
}

/// SplitMix64: `z -> z + C` с последующим финальным микшером.
/// Все умножения и сложения с переполнением по `u64`.
pub fn splitmix64(z: u64) -> u64 {
    let z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Хэш ячейки `(a, b, c)` с подсидом `s`. Отрицательные индексы ячеек
/// приводятся к `u64` побитово (`as`), как требуется в 7.1.
pub fn hash3(s: u64, a: i64, b: i64, c: i64) -> u64 {
    let x0 = splitmix64(s ^ (a as u64).wrapping_mul(K1));
    let x1 = splitmix64(x0 ^ (b as u64).wrapping_mul(K2));
    splitmix64(x1 ^ (c as u64).wrapping_mul(K3))
}

/// Двумерный вариант: третий индекс равен нулю.
pub fn hash2(s: u64, a: i64, b: i64) -> u64 {
    hash3(s, a, b, 0)
}

/// Равномерное число в `[0, 1)` из старших 53 бит хэша.
pub fn unit(h: u64) -> f64 {
    (h >> 11) as f64 * TWO_POW_NEG_53
}

#[cfg(test)]
mod tests {
    use super::{fnv1a64, hash2, hash3, splitmix64, unit};

    #[test]
    fn splitmix64_is_stable_for_zero_and_max() {
        // Значения также лежат в golden/v0/primitives.txt.
        assert_eq!(splitmix64(0), 0xE220_A839_7B1D_CDAF);
        assert_eq!(splitmix64(1), 0x910A_2DEC_8902_5CC1);
        assert_eq!(splitmix64(u64::MAX), 0xE4D9_7177_1B65_2C20);
    }

    #[test]
    fn fnv_differs_for_close_salts() {
        assert_ne!(fnv1a64("plates.seed"), fnv1a64("plates.velocity"));
        assert_eq!(fnv1a64(""), 0xCBF2_9CE4_8422_2325);
    }

    #[test]
    fn hash3_is_deterministic_and_sensitive_to_arguments() {
        assert_eq!(hash3(42, 1, -2, 3), hash3(42, 1, -2, 3));
        assert_ne!(hash3(42, 1, -2, 3), hash3(42, 1, -2, 4));
        assert_eq!(hash2(42, 1, -2), hash3(42, 1, -2, 0));
    }

    #[test]
    fn unit_stays_in_half_open_interval() {
        assert_eq!(unit(0), 0.0);
        assert!(unit(u64::MAX) < 1.0);
        for k in 0..10_000_u64 {
            let value = unit(splitmix64(k));
            assert!(value >= 0.0 && value < 1.0, "unit = {value}");
        }
    }
}
