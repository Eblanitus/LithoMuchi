//! Перевод потенциала рельефа и морфоклиматических зон в C/E/W
//! (PLAN-REVISION 6.4, T122).
//!
//! Собственного `Field::Pv` пока нет (T141), поэтому PV_relief переводится в
//! модуль W на ветви `[0, 2/3]`: `|W| = (PV + 1)/3`. Ванильная производная
//! `peaks_valleys(W)` на этой ветви возвращает тот же PV. Знак W остаётся
//! непрерывным (T102); в холодных горах он смещается к отрицательному
//! (более зубчатые пики), в тёплых — к положительному. Гидрология T115/T116
//! применяется поверх этого перевода и сохраняет русла/болота/дельты.

use crate::climate::ClimateSample;
use crate::math::{Vec2, clamp, clamp01, lerp, smoothstep};
use crate::morphoclimate::{self, MorphoWeights};
use crate::noise::fbm2_unit;
use crate::params::Params;
use crate::relief::{self, ReliefSample};
use crate::seed::{SALT_RELIEF_E, SALT_RELIEF_PV, subseed};
use crate::vanilla::{VanillaSample, weirdness_sign};

/// `base` — ванильные числа T103 без реки; реку накладывает гидрология позже.
/// Поправки затухают на океане через тот же выход из воды, что и слой A.
pub fn apply(
    base: VanillaSample,
    climate: &ClimateSample,
    relief: &ReliefSample,
    weights: &MorphoWeights,
    point: Vec2,
    seed: u64,
    params: &Params,
) -> VanillaSample {
    apply_with_pv(base, climate, relief, weights, point, seed, params).0
}

/// T141: тот же перевод T122, но отдаёт и собственный PV до среза руслами.
/// Генератор хранит его в `WorldSample.pv`; ванильный W остаётся непрерывным.
pub fn apply_with_pv(
    base: VanillaSample,
    climate: &ClimateSample,
    relief: &ReliefSample,
    weights: &MorphoWeights,
    point: Vec2,
    seed: u64,
    params: &Params,
) -> (VanillaSample, f64) {
    debug_assert!(relief::validate(params).is_ok());
    debug_assert!(morphoclimate::validate(params).is_ok());
    let c = &params.climate;
    let m = &params.morpho;
    let r = &params.relief;
    let surface = climate.surface;
    let land = smoothstep(0.0, r.land_e_fade_km, surface.e_km);
    let zone = morphoclimate::mix(weights, params);
    let a = smoothstep(r.a_lo, r.a_hi, relief.a);

    // E: равнины → холмы по A и зонам, затем горы и плато как в 7.9.2.
    let lambda_e = params.block.lambda_e_blocks;
    let n_e = fbm2_unit(
        subseed(seed, SALT_RELIEF_E),
        point.x / lambda_e,
        point.z / lambda_e,
        1.0,
        3,
        0.5,
    );
    let e_relief = lerp(zone.e_flat, zone.e_hill, a) + zone.sigma_e * n_e;
    let m_e = smoothstep(c.m_e_lo, c.m_e_hi, surface.mountain);
    let mountains = c.e_mnt + c.e_mnt_noise * n_e;
    let e_relief = lerp(e_relief, mountains, m_e);
    let e_relief = lerp(
        e_relief,
        c.e_plateau,
        clamp01(surface.plateau * (1.0 - m_e)),
    );
    let e_value = clamp(lerp(base.e, e_relief, land), -1.0, 1.0);

    // PV_relief → |W| на ветви [0, 2/3]; знак непрерывный, горы смещают его.
    // Шум считается на фиксированной λ каждой зоны и смешивается весами:
    // если подставлять смесь λ в координату шума, медленный уход λ даёт
    // фазовый набег вдали от начала координат и лишние скачки PV.
    let lambda_factor = if params.mode == crate::mode::Mode::Scientific {
        m.scientific_lambda_factor
    } else {
        1.0
    };
    let sigma_factor = if params.mode == crate::mode::Mode::Scientific {
        m.scientific_sigma_factor
    } else {
        1.0
    };
    let seed_pv = subseed(seed, SALT_RELIEF_PV);
    let mut pv = 0.0;
    for (k, w) in weights.values.iter().enumerate() {
        if *w <= 0.0 {
            continue;
        }
        let lambda = (m.lambda_pv_blocks[k] * lambda_factor).max(1.0);
        let n_pv = fbm2_unit(seed_pv, point.x / lambda, point.z / lambda, 1.0, 3, 0.5);
        pv += w * (lerp(m.pv_flat[k], m.pv_hill[k], a) + m.sigma_pv[k] * sigma_factor * n_pv);
    }
    let pv = clamp(pv, m.pv_min, m.pv_max);
    let magnitude = (pv + 1.0) / 3.0;
    let mountain_weight = clamp01(weights.values[6] + weights.values[7]);
    let sign_bias = weights.values[7] - weights.values[6];
    let sign_noise = weirdness_sign(point.x, point.z, seed, params);
    let sign = clamp(sign_noise * (1.0 - mountain_weight) + sign_bias, -1.0, 1.0);
    let w_relief = sign * magnitude;
    let w_value = clamp(lerp(base.w, w_relief, land), -1.0, 1.0);

    // C: котловины ниже, нагорья выше; только суша, океанский сплайн не трогаем.
    let c_value = clamp(
        base.c + land * (m.c_highland_gain * relief.a - m.c_basin_drop * relief.basin),
        -1.0,
        1.0,
    );

    let vanilla = VanillaSample {
        c: c_value,
        e: e_value,
        w: w_value,
        t: base.t,
        h: base.h,
    };
    (vanilla, pv)
}

#[cfg(test)]
mod tests {
    use super::apply;
    use crate::climate::{
        ClimateSample, PoleSample, PrecipSample, SurfaceSample, TemperatureSample,
    };
    use crate::math::Vec2;
    use crate::mode::Mode;
    use crate::morphoclimate::{MorphoWeights, weights};
    use crate::params::Params;
    use crate::relief::ReliefSample;
    use crate::vanilla::{VanillaSample, peaks_valleys};
    use crate::version::GenVersion;

    fn params() -> Params {
        Params::defaults(GenVersion::V1Dev, Mode::Gameplay)
    }

    fn climate(e_km: f64, mountain: f64, t_bio: f64) -> ClimateSample {
        ClimateSample {
            point: Vec2::default(),
            surface: SurfaceSample {
                e_km,
                iota_l: e_km.max(0.0),
                mountain,
                plateau: 0.0,
                ..SurfaceSample::default()
            },
            pole: PoleSample {
                phi_deg: 40.0,
                north: Vec2::new(0.0, -1.0),
                east: Vec2::new(1.0, 0.0),
                pole: Vec2::default(),
                distance_l: 1.0,
            },
            wind: Vec2::new(1.0, 0.0),
            temperature: TemperatureSample {
                t_c: t_bio,
                sst_c: t_bio,
                amp_c: 8.0,
                t_bio,
                t_min: t_bio - 8.0,
            },
            precip: PrecipSample {
                mu: 0.5,
                cont: 0.5,
                p_mm: 1000.0,
                pet_mm: 600.0,
                ratio: 0.6,
                slope: 0.0,
            },
        }
    }

    fn base() -> VanillaSample {
        VanillaSample {
            c: 0.3,
            e: 0.2,
            w: 0.3,
            t: 0.0,
            h: 0.0,
        }
    }

    #[test]
    fn ocean_keeps_base_fields_and_land_changes_them() {
        let p = params();
        let w = weights(10.0, 0.6, 0.0, &p);
        let relief = ReliefSample {
            a: 0.0,
            basin: 0.5,
            ..ReliefSample::default()
        };
        let sea = apply(
            base(),
            &climate(-1.0, 0.0, 10.0),
            &relief,
            &w,
            Vec2::new(100.0, -50.0),
            42,
            &p,
        );
        assert_eq!(sea, base());
        let land = apply(
            base(),
            &climate(1.0, 0.0, 10.0),
            &relief,
            &w,
            Vec2::new(100.0, -50.0),
            42,
            &p,
        );
        assert!(land.c < base().c, "котловина должна понижать C");
        assert!((land.e - base().e).abs() > 1e-6);
        assert!((land.w - base().w).abs() > 1e-6);
        for value in [land.c, land.e, land.w] {
            assert!((-1.0..=1.0).contains(&value));
        }
    }

    #[test]
    fn w_magnitude_inverts_to_pv_relief_on_the_branch() {
        let p = params();
        let w = weights(10.0, 0.6, 0.0, &p);
        let relief = ReliefSample {
            a: 0.5,
            ..ReliefSample::default()
        };
        let out = apply(
            base(),
            &climate(1.0, 0.0, 10.0),
            &relief,
            &w,
            Vec2::new(123.0, 456.0),
            42,
            &p,
        );
        let pv = peaks_valleys(out.w);
        let a = crate::math::smoothstep(p.relief.a_lo, p.relief.a_hi, relief.a);
        let mut expected = 0.0;
        for (k, weight) in w.values.iter().enumerate() {
            let lambda = p.morpho.lambda_pv_blocks[k];
            let n_pv = crate::noise::fbm2_unit(
                crate::seed::subseed(42, crate::seed::SALT_RELIEF_PV),
                123.0 / lambda,
                456.0 / lambda,
                1.0,
                3,
                0.5,
            );
            expected += weight
                * (crate::math::lerp(p.morpho.pv_flat[k], p.morpho.pv_hill[k], a)
                    + p.morpho.sigma_pv[k] * n_pv);
        }
        let expected = crate::math::clamp(expected, p.morpho.pv_min, p.morpho.pv_max);
        assert!((pv - expected).abs() < 1e-12, "pv={pv} expected={expected}");
        assert!(out.w.abs() <= 2.0 / 3.0 + 1e-12);
    }

    #[test]
    fn cold_mountains_take_negative_sign_and_warm_positive() {
        let p = params();
        let relief = ReliefSample {
            a: 0.9,
            ..ReliefSample::default()
        };
        let mut cold = MorphoWeights::default();
        cold.values[6] = 1.0;
        let mut warm = MorphoWeights::default();
        warm.values[7] = 1.0;
        let c = apply(
            base(),
            &climate(3.0, 1.0, -5.0),
            &relief,
            &cold,
            Vec2::new(10.0, 20.0),
            42,
            &p,
        );
        let w = apply(
            base(),
            &climate(3.0, 1.0, 25.0),
            &relief,
            &warm,
            Vec2::new(10.0, 20.0),
            42,
            &p,
        );
        assert!(c.w < 0.0);
        assert!(w.w > 0.0);
    }

    #[test]
    fn fields_are_continuous_across_the_land_gate() {
        let p = params();
        let w = weights(10.0, 0.6, 0.2, &p);
        let relief = ReliefSample {
            a: 0.6,
            basin: 0.2,
            ..ReliefSample::default()
        };
        let mut previous: Option<VanillaSample> = None;
        for k in 0..200 {
            let e_km = -0.02 + k as f64 * 1e-5;
            let out = apply(
                base(),
                &climate(e_km, 0.2, 10.0),
                &relief,
                &w,
                Vec2::new(300.0, -200.0),
                42,
                &p,
            );
            if let Some(prev) = previous {
                assert!((out.c - prev.c).abs() < 1e-3);
                assert!((out.e - prev.e).abs() < 1e-3);
                assert!((out.w - prev.w).abs() < 1e-3);
            }
            previous = Some(out);
        }
    }
}
