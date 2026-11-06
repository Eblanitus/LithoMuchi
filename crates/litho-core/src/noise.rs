//! Градиентный шум, fBm, гребневый шум и искажение домена (план 7.2).
//!
//! Таблица направлений и углы поворотов октав — константы в `noise_tables.rs`.
//! Никаких вызовов `sin`/`cos` во время работы генератора.

use crate::hash::{hash3, splitmix64, unit};
use crate::math::{SQRT_2, Vec2, clamp, lerp};
use crate::noise_tables::{DIRS, OCTAVES, ROT};

/// Квинтическая сглаживающая функция `t³(t(6t − 15) + 10)`.
fn quintic(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn gradient(seed: u64, i: i64, j: i64) -> [f64; 2] {
    let index = (hash3(seed, i, j, 0) & 255) as usize;
    DIRS[index]
}

/// Градиентный шум Перлина на плоскости. Значение обрезано до `[−1, 1]`.
pub fn noise2(seed: u64, x: f64, z: f64) -> f64 {
    let x0 = x.floor();
    let z0 = z.floor();
    let fx = x - x0;
    let fz = z - z0;
    let i = x0 as i64;
    let j = z0 as i64;
    let u = quintic(fx);
    let v = quintic(fz);

    let g00 = gradient(seed, i, j);
    let g10 = gradient(seed, i + 1, j);
    let g01 = gradient(seed, i, j + 1);
    let g11 = gradient(seed, i + 1, j + 1);

    let n00 = g00[0] * fx + g00[1] * fz;
    let n10 = g10[0] * (fx - 1.0) + g10[1] * fz;
    let n01 = g01[0] * fx + g01[1] * (fz - 1.0);
    let n11 = g11[0] * (fx - 1.0) + g11[1] * (fz - 1.0);

    let a = lerp(n00, n10, u);
    let b = lerp(n01, n11, u);
    clamp(lerp(a, b, v) * SQRT_2, -1.0, 1.0)
}

/// Сумма октав: каждая следующая вдвое мельче и слабее в `gain` раз.
/// Октавы повёрнуты на угол `0.61·k` и сдвинуты, чтобы не было выравнивания
/// по осям (7.2). Число октав ограничено таблицей `ROT`.
pub fn fbm2(seed: u64, x: f64, z: f64, lambda0: f64, octaves: usize, gain: f64) -> f64 {
    let count = octaves.min(OCTAVES);
    let mut sum = 0.0;
    let mut norm = 0.0;
    let mut amplitude = 1.0;
    let mut lambda = lambda0;
    for (k, rot) in ROT.iter().enumerate().take(count) {
        let octave_seed = splitmix64(seed.wrapping_add(k as u64));
        let ox = unit(hash3(seed, k as i64, 0, 1)) * 1024.0;
        let oz = unit(hash3(seed, k as i64, 1, 1)) * 1024.0;
        let px = rot[0] * x - rot[1] * z;
        let pz = rot[1] * x + rot[0] * z;
        sum += amplitude * noise2(octave_seed, px / lambda + ox, pz / lambda + oz);
        norm += amplitude;
        amplitude *= gain;
        lambda *= 0.5;
    }
    if norm > 0.0 { sum / norm } else { 0.0 }
}

/// Гребневый шум: острые гребни для структуры хребтов.
pub fn ridged2(seed: u64, x: f64, z: f64, lambda0: f64, octaves: usize, gain: f64) -> f64 {
    let count = octaves.min(OCTAVES);
    let mut sum = 0.0;
    let mut norm = 0.0;
    let mut amplitude = 1.0;
    let mut lambda = lambda0;
    for (k, rot) in ROT.iter().enumerate().take(count) {
        let octave_seed = splitmix64(seed.wrapping_add(k as u64));
        let px = rot[0] * x - rot[1] * z;
        let pz = rot[1] * x + rot[0] * z;
        let n = noise2(octave_seed, px / lambda, pz / lambda);
        let ridge = 1.0 - n.abs();
        sum += amplitude * ridge * ridge;
        norm += amplitude;
        amplitude *= gain;
        lambda *= 0.5;
    }
    if norm > 0.0 { sum / norm } else { 0.0 }
}

/// Искажение домена: две независимые fBm по осям, амплитуда первой октавы
/// `amplitude`. Возвращает сдвиг точки в макро-единицах.
pub fn warp2(
    seed_x: u64,
    seed_z: u64,
    point: Vec2,
    lambda0: f64,
    amplitude: f64,
    octaves: usize,
    gain: f64,
) -> Vec2 {
    Vec2::new(
        amplitude * fbm2(seed_x, point.x, point.z, lambda0, octaves, gain),
        amplitude * fbm2(seed_z, point.x, point.z, lambda0, octaves, gain),
    )
}

#[cfg(test)]
mod tests {
    use super::{DIRS, fbm2, noise2, ridged2, warp2};
    use crate::math::Vec2;

    #[test]
    fn directions_are_unit_vectors() {
        for dir in DIRS.iter() {
            let len2 = dir[0] * dir[0] + dir[1] * dir[1];
            assert!((len2 - 1.0).abs() < 1e-12, "len2 = {len2}");
        }
    }

    #[test]
    fn noise_stays_in_range() {
        for k in 0..2000 {
            let x = k as f64 * 0.137 - 30.0;
            let z = k as f64 * -0.071 + 12.0;
            let n = noise2(1234, x, z);
            assert!((-1.0..=1.0).contains(&n), "noise = {n}");
        }
    }

    #[test]
    fn noise_is_continuous() {
        for k in 0..1000 {
            let x = k as f64 * 0.031;
            let z = k as f64 * 0.017;
            let a = noise2(77, x, z);
            let b = noise2(77, x + 0.001, z);
            assert!((a - b).abs() < 0.05, "delta = {}", (a - b).abs());
        }
    }

    #[test]
    fn fbm_and_ridged_are_bounded() {
        for k in 0..500 {
            let x = k as f64 * 0.21;
            let z = k as f64 * 0.13;
            let f = fbm2(9, x, z, 24.0, 4, 0.45);
            let r = ridged2(9, x, z, 24.0, 3, 0.45);
            assert!((-1.0..=1.0).contains(&f), "fbm = {f}");
            assert!((0.0..=1.0).contains(&r), "ridged = {r}");
        }
    }

    #[test]
    fn warp_is_bounded_by_amplitude() {
        for k in 0..200 {
            let p = Vec2::new(k as f64 * 0.7, k as f64 * -0.3);
            let w = warp2(1, 2, p, 24.0, 1.6, 4, 0.45);
            assert!(w.x.abs() <= 1.6 + 1e-12, "w.x = {}", w.x);
            assert!(w.z.abs() <= 1.6 + 1e-12, "w.z = {}", w.z);
        }
    }
}
