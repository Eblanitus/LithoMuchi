//! Перевод наших полей в пять ванильных параметров C, E, W, T, H (план 7.9,
//! задачи T41–T43). Главное правило: C, E и W непрерывны, T и H могут быть
//! ступенчатыми.

use crate::classify::ClassSample;
use crate::climate::ClimateSample;
use crate::math::{clamp, clamp01, lerp, piecewise, smoothstep};
use crate::noise::{fbm2, fbm2_unit, ridged2};
use crate::params::Params;
use crate::seed::{SALT_VANILLA_E, SALT_VANILLA_S, SALT_VANILLA_W, subseed};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VanillaSample {
    pub c: f64,
    pub e: f64,
    pub w: f64,
    pub t: f64,
    pub h: f64,
}

/// Амплитудный fBm пяти полей. Только v1-dev читает amplitude как σ.
/// Соли, октавы, gain и порядок операций v0 не меняются.
pub(crate) fn amplitude_fbm(seed: u64, x: f64, z: f64, lambda: f64, params: &Params) -> f64 {
    if params.version.is_water_relief_dev() {
        fbm2_unit(seed, x, z, lambda, 3, 0.5)
    } else {
        fbm2(seed, x, z, lambda, 3, 0.5)
    }
}

/// Ванильная производная PV от W. Собственного Field::Pv пока нет (T141).
pub fn peaks_valleys(w: f64) -> f64 {
    1.0 - (3.0 * w.abs() - 2.0).abs()
}

/// Континентальность C: на море по глубине, на суше по удалённости от берега.
/// На самом берегу значение одинаково с обеих сторон (7.9.1).
pub fn continentalness(e_km: f64, iota_l: f64, params: &Params) -> f64 {
    let c = &params.climate;
    if e_km < 0.0 {
        piecewise(&c.c_sea_e_km, &c.c_sea, e_km)
    } else {
        let inland = piecewise(&c.c_land_iota_l, &c.c_land, iota_l);
        lerp(c.c_coast, inland, smoothstep(0.0, c.c_coast_e_km, e_km))
    }
}

/// Эрозия E: равнины → горы → плато, всё непрерывно (7.9.2).
pub fn erosion(x: f64, z: f64, mountain: f64, plateau: f64, seed: u64, params: &Params) -> f64 {
    let c = &params.climate;
    let lambda = params.block.lambda_e_blocks;
    let n = amplitude_fbm(
        subseed(seed, SALT_VANILLA_E),
        x / lambda,
        z / lambda,
        1.0,
        params,
    );
    let m_e = smoothstep(c.m_e_lo, c.m_e_hi, mountain);
    let base = c.e_base + c.e_noise * n;
    let mountains = c.e_mnt + c.e_mnt_noise * n;
    let value = lerp(base, mountains, m_e);
    let result = lerp(value, c.e_plateau, clamp01(plateau * (1.0 - m_e)));
    if params.version.is_water_relief_dev() {
        clamp(result, -1.0, 1.0)
    } else {
        result
    }
}

/// Странность W: модуль задаёт срез рельефа, знак — вариант биома (7.9.3).
/// `r` — сила реки. В v0 это локальные реки быстрого пути; сеть стока — T110.
pub fn weirdness(
    x: f64,
    z: f64,
    mountain: f64,
    plateau: f64,
    seed: u64,
    params: &Params,
    r: f64,
) -> f64 {
    let c = &params.climate;
    let lambda = params.block.lambda_w_blocks;
    let sub_e = subseed(seed, SALT_VANILLA_W);
    let n = amplitude_fbm(sub_e, x / lambda, z / lambda, 1.0, params);
    let ridge = ridged2(sub_e, x / lambda, z / lambda, 1.0, 3, 0.5);
    let m_plain = c.w_plain_base + c.w_plain_noise * n;
    let m_mnt = c.w_mnt_base + c.w_mnt_noise * ridge;
    let m_e = smoothstep(0.1, 0.5, mountain);
    let mut m = lerp(m_plain, m_mnt, m_e);
    m = lerp(m, c.w_plateau, clamp01(plateau * (1.0 - m_e)));
    m = m.min(c.w_max);
    let sign = if params.version.is_water_relief_dev() {
        // f/g — приближённое расстояние до нуля f в блоках. Центральные
        // разности берутся с шагом ровно 1 блок, а не с шагом coarse-решётки.
        m = clamp(m, 0.0, c.w_max);
        let lambda_s = params.block.lambda_s_blocks;
        let seed_s = subseed(seed, SALT_VANILLA_S);
        let f = amplitude_fbm(seed_s, x / lambda_s, z / lambda_s, 1.0, params);
        let dx = amplitude_fbm(seed_s, (x + 1.0) / lambda_s, z / lambda_s, 1.0, params)
            - amplitude_fbm(seed_s, (x - 1.0) / lambda_s, z / lambda_s, 1.0, params);
        let dz = amplitude_fbm(seed_s, x / lambda_s, (z + 1.0) / lambda_s, 1.0, params)
            - amplitude_fbm(seed_s, x / lambda_s, (z - 1.0) / lambda_s, 1.0, params);
        let gradient = (dx * dx + dz * dz).sqrt() * lambda_s * 0.5;
        let denominator = (gradient * params.block.w_sign_half_width_blocks).max(c.w_sign_epsilon);
        clamp(f / denominator, -1.0, 1.0)
    } else {
        clamp(
            c.w_sign_scale
                * fbm2(
                    subseed(seed, SALT_VANILLA_S),
                    x / params.block.lambda_s_blocks,
                    z / params.block.lambda_s_blocks,
                    1.0,
                    3,
                    0.5,
                ),
            -1.0,
            1.0,
        )
    };
    let result = sign * m * (1.0 - clamp01(r));
    if params.version.is_water_relief_dev() {
        clamp(result, -1.0, 1.0)
    } else {
        result
    }
}

/// Пять чисел для ванили. `t` и `h` — центры полос, остальные непрерывны.
pub fn sample(
    climate: &ClimateSample,
    class: &ClassSample,
    x: f64,
    z: f64,
    seed: u64,
    params: &Params,
    river: f64,
) -> VanillaSample {
    let c = &params.climate;
    let surface = climate.surface;
    let c_value = continentalness(surface.e_km, surface.iota_l, params);
    let e_value = erosion(x, z, surface.mountain, surface.plateau, seed, params);
    let w_value = weirdness(x, z, surface.mountain, surface.plateau, seed, params, river);
    let t_value = c.t_centers[(class.cell.t as usize).min(4)];
    let h_value = if class.is_ocean {
        0.0
    } else if class.cell.h == 4 {
        c.h4_base + c.h4_gain * smoothstep(0.5, 0.25, climate.precip.ratio)
    } else {
        c.h_centers[(class.cell.h as usize).min(4)]
    };
    VanillaSample {
        c: c_value,
        e: e_value,
        w: w_value,
        t: t_value,
        h: h_value,
    }
}

#[cfg(test)]
mod tests {
    use super::{continentalness, erosion, sample, weirdness};
    use crate::classify::classify_inputs;
    use crate::climate::{ClimateSample, PrecipSample, SurfaceSample, TemperatureSample};
    use crate::math::{Vec2, clamp01, smoothstep};
    use crate::mode::Mode;
    use crate::params::Params;
    use crate::version::GenVersion;

    fn params() -> Params {
        Params::defaults(GenVersion::V0, Mode::Gameplay)
    }

    #[test]
    fn continentalness_matches_plan_knots_and_is_continuous_at_coast() {
        let p = params();
        assert!((continentalness(0.0, 0.0, &p) - (-0.17)).abs() < 1e-12);
        assert!((continentalness(-0.000_001, 0.0, &p) - (-0.17)).abs() < 1e-5);
        assert!((continentalness(-4.5, 0.0, &p) - (-0.70)).abs() < 1e-12);
        let ridge = continentalness(-2.3, 0.0, &p);
        assert!((ridge + 0.44).abs() < 0.03, "гребень СОХ: {ridge}");
        assert!((continentalness(0.6, 20.0, &p) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn c_e_w_are_continuous_on_a_smooth_transect() {
        let p = params();
        let mut previous: Option<(f64, f64, f64)> = None;
        for k in 0..2000 {
            let t = k as f64 * 0.01;
            let x = t * 120.0;
            let e_km = -5.0 + 10.0 * smoothstep(0.0, 20.0, t);
            let mountain = clamp01((e_km - 0.2) / 3.0);
            let plateau = 0.3;
            let c = continentalness(e_km, t * 0.5, &p);
            let e = erosion(x, x * 0.3, mountain, plateau, 42, &p);
            let w = weirdness(x, x * 0.3, mountain, plateau, 42, &p, 0.0);
            assert!(c.is_finite() && e.is_finite() && w.is_finite());
            if let Some((pc, pe, pw)) = previous {
                assert!((c - pc).abs() < 0.1, "скачок C на t={t}: {}", c - pc);
                assert!((e - pe).abs() < 0.1, "скачок E на t={t}: {}", e - pe);
                assert!((w - pw).abs() < 0.1, "скачок W на t={t}: {}", w - pw);
            }
            previous = Some((c, e, w));
        }
    }

    #[test]
    fn all_five_values_stay_in_vanilla_range() {
        let p = params();
        for k in 0..200 {
            let t = k as f64 * 0.07;
            let surface = SurfaceSample {
                e_km: -4.0 + 8.0 * smoothstep(0.0, 14.0, t),
                iota_l: t,
                mountain: clamp01(t / 10.0),
                plateau: 0.2,
                boundary_weights: [0.0; 7],
                sigma_l: 0.0,
            };
            let climate = ClimateSample {
                point: Vec2::new(t, -t),
                surface,
                pole: crate::climate::PoleSample {
                    phi_deg: 40.0,
                    north: Vec2::new(0.0, -1.0),
                    east: Vec2::new(1.0, 0.0),
                    pole: Vec2::default(),
                    distance_l: 1.0,
                },
                wind: Vec2::new(0.3, 0.2),
                temperature: TemperatureSample {
                    t_c: 10.0,
                    sst_c: 12.0,
                    amp_c: 8.0,
                    t_bio: 10.0,
                    t_min: 2.0,
                },
                precip: PrecipSample {
                    mu: 0.5,
                    cont: 0.4,
                    p_mm: 900.0,
                    pet_mm: 589.3,
                    ratio: 0.65,
                    slope: 0.0,
                },
            };
            let class = classify_inputs(10.0, 0.65, 2.0, &p);
            let v = sample(&climate, &class, t * 100.0, -t * 50.0, 42, &p, 0.0);
            for value in [v.c, v.e, v.w, v.t, v.h] {
                assert!(
                    (-1.0..=1.0).contains(&value),
                    "значение {value} вне [-1, 1]"
                );
            }
        }
    }
}
