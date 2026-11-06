//! Сдвиг точки появления (план 7.12, задача T54).
//! p* — функция только сида и версии: оба режима начинаются в одном месте
//! планеты. В v0 сдвиг посчитан и доступен, но ещё не подставлен в
//! макро-координаты (Д-015): карты песочницы остаются на прежнем месте.

use crate::climate::{dryness, pole_sample, precipitation, temperature};
use crate::crust::crust_from_sample;
use crate::hash::{hash3, unit};
use crate::math::{Vec2, smoothstep};
use crate::params::Params;
use crate::plates::plate_query;
use crate::seed::{SALT_SPAWN, subseed};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpawnQuality {
    pub e_km: f64,
    pub kappa_l: f64,
    pub mountain: f64,
    pub phi_deg: f64,
    pub ratio: f64,
    pub score: f64,
    pub good: bool,
}

/// Насколько точка подходит для появления по критериям 7.12.
pub fn evaluate(point: Vec2, seed: u64, params: &Params) -> SpawnQuality {
    let s = &params.spawn;
    let Some(plates) = plate_query(point, seed, params) else {
        return SpawnQuality {
            e_km: -1.0,
            kappa_l: -10.0,
            mountain: 1.0,
            phi_deg: 0.0,
            ratio: 100.0,
            score: 0.0,
            good: false,
        };
    };
    let crust = crust_from_sample(point, &plates, seed, params);
    let phi_deg = pole_sample(point, seed, params).map_or(0.0, |p| p.phi_deg);
    // Луч влаги здесь не считаем: для поиска довольно консервативной оценки
    // осадков по поясу при средней влажности (Д-015).
    let p_mm = precipitation(phi_deg, s.fixed_mu, s.fixed_wind, 0.0, params);
    let t = temperature(phi_deg, crust.e_km, 0.5, params);
    let (_, ratio) = dryness(t.t_bio, p_mm, params);

    let e_ok = crust.e_km >= s.min_e_km && crust.e_km <= s.max_e_km;
    let kappa_ok = crust.kappa_l >= s.min_kappa_l;
    let mountain_ok = crust.mountain < s.max_mountain;
    let phi_ok = phi_deg >= s.min_phi_deg && phi_deg <= s.max_phi_deg;
    let ratio_ok = ratio >= s.min_ratio && ratio <= s.max_ratio;

    let soft_e = smoothstep(s.min_e_km - 0.5, s.min_e_km, crust.e_km)
        * (1.0 - smoothstep(s.max_e_km, s.max_e_km + 0.5, crust.e_km));
    let soft_k = smoothstep(s.min_kappa_l - 2.0, s.min_kappa_l, crust.kappa_l);
    let soft_m = 1.0 - smoothstep(s.max_mountain, s.max_mountain + 0.15, crust.mountain);
    let soft_phi = smoothstep(s.min_phi_deg - 8.0, s.min_phi_deg, phi_deg)
        * (1.0 - smoothstep(s.max_phi_deg, s.max_phi_deg + 8.0, phi_deg));
    let soft_r = smoothstep(s.min_ratio * 0.5, s.min_ratio, ratio)
        * (1.0 - smoothstep(s.max_ratio, s.max_ratio * 2.0, ratio));

    SpawnQuality {
        e_km: crust.e_km,
        kappa_l: crust.kappa_l,
        mountain: crust.mountain,
        phi_deg,
        ratio,
        score: soft_e * soft_k * soft_m * soft_phi * soft_r,
        good: e_ok && kappa_ok && mountain_ok && phi_ok && ratio_ok,
    }
}

/// Первый годный кандидат; если среди случайных точек его нет — детерминированная
/// сетка и, наконец, короткий поиск вокруг лучшей точки.
pub fn find_shift(seed: u64, params: &Params) -> Vec2 {
    let s = &params.spawn;
    let sub = subseed(seed, SALT_SPAWN);
    let mut best = (Vec2::default(), -1.0_f64);
    for k in 0..s.candidates {
        let h = hash3(sub, k as i64, 0, 0);
        let h2 = hash3(sub, k as i64, 0, 1);
        let point = Vec2::new(
            (unit(h) * 2.0 - 1.0) * s.radius_l,
            (unit(h2) * 2.0 - 1.0) * s.radius_l,
        );
        let quality = evaluate(point, seed, params);
        if quality.good {
            return point;
        }
        if quality.score > best.1 {
            best = (point, quality.score);
        }
    }
    let n = s.fallback_grid.max(2) as f64;
    for i in 0..s.fallback_grid {
        for j in 0..s.fallback_grid {
            let point = Vec2::new(
                (2.0 * i as f64 / (n - 1.0) - 1.0) * s.radius_l,
                (2.0 * j as f64 / (n - 1.0) - 1.0) * s.radius_l,
            );
            let quality = evaluate(point, seed, params);
            if quality.good {
                return point;
            }
            if quality.score > best.1 {
                best = (point, quality.score);
            }
        }
    }
    // Короткий поиск рядом с лучшей точкой: климат меняется плавно, поэтому
    // хорошее место почти всегда лежит в паре L от неё.
    // Направления фиксированы таблицей, без системных тригонометрических
    // функций в местах, от которых зависит выбор точки.
    let half_sqrt2 = std::f64::consts::FRAC_1_SQRT_2;
    let dirs = [
        (1.0_f64, 0.0_f64),
        (0.923_879_5, 0.382_683_4),
        (half_sqrt2, half_sqrt2),
        (0.382_683_4, 0.923_879_5),
        (0.0, 1.0),
        (-0.382_683_4, 0.923_879_5),
        (-half_sqrt2, half_sqrt2),
        (-0.923_879_5, 0.382_683_4),
        (-1.0, 0.0),
        (-0.923_879_5, -0.382_683_4),
        (-half_sqrt2, -half_sqrt2),
        (-0.382_683_4, -0.923_879_5),
        (0.0, -1.0),
        (0.382_683_4, -0.923_879_5),
        (half_sqrt2, -half_sqrt2),
        (0.923_879_5, -0.382_683_4),
    ];
    for ring in 1..=6 {
        let radius = ring as f64 * 0.4;
        for (dx, dz) in dirs {
            let point = best.0.add(Vec2::new(dx, dz).scale(radius));
            let quality = evaluate(point, seed, params);
            if quality.good {
                return point;
            }
        }
    }
    best.0
}

#[cfg(test)]
mod tests {
    use super::{evaluate, find_shift};
    use crate::mode::Mode;
    use crate::params::Params;
    use crate::version::GenVersion;

    #[test]
    fn spawn_point_is_good_on_one_hundred_seeds() {
        let params = Params::defaults(GenVersion::V0, Mode::Gameplay);
        for seed in 0..100_u64 {
            let shift = find_shift(seed, &params);
            assert!(shift.x.is_finite() && shift.z.is_finite(), "сид {seed}");
            let quality = evaluate(shift, seed, &params);
            assert!(
                quality.good,
                "сид {seed}: e={e} κ={k} M={m} φ={phi} R={r}",
                e = quality.e_km,
                k = quality.kappa_l,
                m = quality.mountain,
                phi = quality.phi_deg,
                r = quality.ratio,
            );
        }
    }

    #[test]
    fn same_seed_gives_same_shift() {
        let params = Params::defaults(GenVersion::V0, Mode::Gameplay);
        assert_eq!(find_shift(42, &params), find_shift(42, &params));
    }
}
