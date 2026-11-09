//! Морфоклиматические зоны (PLAN-REVISION 6.3, T121).
//!
//! Зоны задаются по `T_bio`, индексу сухости `R = PET/P` и горности `M`.
//! Веса плавные и образуют разбиение единицы: дискретных переключателей нет,
//! поэтому E/PV/C в T122 смогут смешивать параметры зон без швов. Числа зон
//! стартовые; подбор гипсометрии и долей остаётся T123.

use crate::math::{clamp01, smoothstep};
use crate::mode::Mode;
use crate::params::{MORPHO_ZONES, Params};

pub const ZONE_NAMES: [&str; MORPHO_ZONES] = [
    "polar",
    "boreal",
    "temperate_humid",
    "semiarid",
    "arid",
    "tropical_humid",
    "mountain_cold",
    "mountain_warm",
];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MorphoWeights {
    pub values: [f64; MORPHO_ZONES],
}

impl MorphoWeights {
    pub fn sum(&self) -> f64 {
        self.values.iter().sum()
    }

    /// Только диагностика: максимальный вес, ничьи — по порядку зон.
    pub fn dominant(&self) -> usize {
        let mut best = 0;
        for (k, value) in self.values.iter().enumerate() {
            if *value > self.values[best] {
                best = k;
            }
        }
        best
    }
}

/// Смесь параметров зон по весам. `lambda_pv_blocks` и σ приводятся к режиму;
/// E/PV самих зон пока одинаковы в обоих режимах (подбор T123).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MorphoMix {
    pub e_flat: f64,
    pub e_hill: f64,
    pub pv_flat: f64,
    pub pv_hill: f64,
    pub lambda_pv_blocks: f64,
    pub sigma_e: f64,
    pub sigma_pv: f64,
}

pub fn validate(params: &Params) -> Result<(), &'static str> {
    if !params.version.is_water_relief_dev() {
        return Err("морфоклиматические зоны доступны только в v1-dev");
    }
    params.morpho.validate()
}

pub fn weights(t_bio: f64, ratio: f64, mountain: f64, params: &Params) -> MorphoWeights {
    debug_assert!(validate(params).is_ok());
    let p = &params.morpho;
    let t = t_bio;
    let r = if ratio.is_finite() {
        ratio.max(0.0)
    } else {
        0.0
    };
    let m = clamp01(mountain);

    // Температурные полосы: сумма четырёх членов равна 1 при непересекающихся
    // переходах (validate следит, что fade не шире половины зазора).
    let polar = 1.0 - smoothstep(p.t_polar_c - p.t_fade_c, p.t_polar_c + p.t_fade_c, t);
    let boreal = smoothstep(p.t_polar_c - p.t_fade_c, p.t_polar_c + p.t_fade_c, t)
        * (1.0 - smoothstep(p.t_boreal_c - p.t_fade_c, p.t_boreal_c + p.t_fade_c, t));
    let temperate = smoothstep(p.t_boreal_c - p.t_fade_c, p.t_boreal_c + p.t_fade_c, t)
        * (1.0 - smoothstep(p.t_tropical_c - p.t_fade_c, p.t_tropical_c + p.t_fade_c, t));
    let tropical = smoothstep(p.t_tropical_c - p.t_fade_c, p.t_tropical_c + p.t_fade_c, t);

    // Сухость: humid1 + semiarid + arid = 1. Влажная тропика забирает R<0,5;
    // тропики с R 0,5…1 остаются в умеренно-влажной морфологии без ступени.
    let humid05 = 1.0
        - smoothstep(
            p.r_tropical_humid - p.r_fade,
            p.r_tropical_humid + p.r_fade,
            r,
        );
    let humid1 = 1.0 - smoothstep(p.r_humid - p.r_fade, p.r_humid + p.r_fade, r);
    let semiarid_dry = smoothstep(p.r_humid - p.r_fade, p.r_humid + p.r_fade, r)
        * (1.0 - smoothstep(p.r_arid - p.r_fade, p.r_arid + p.r_fade, r));
    let arid_dry = smoothstep(p.r_arid - p.r_fade, p.r_arid + p.r_fade, r);
    let noncold = temperate + tropical;
    let subhumid = (humid1 - humid05).max(0.0);

    let mut values = [0.0; MORPHO_ZONES];
    values[0] = polar;
    values[1] = boreal;
    values[2] = temperate * humid1 + tropical * subhumid;
    values[3] = noncold * semiarid_dry;
    values[4] = noncold * arid_dry;
    values[5] = tropical * humid05;

    // Горы перекрывают климатическую зону плавно по M; холодные/тёплые пики
    // различаются только знаком W/jaggedness в T122, не дискретным типом.
    let mountain_weight = smoothstep(p.mountain_lo, p.mountain_hi, m);
    let cold = 1.0
        - smoothstep(
            p.mountain_t_split_c - p.mountain_t_fade_c,
            p.mountain_t_split_c + p.mountain_t_fade_c,
            t,
        );
    for value in values.iter_mut() {
        *value *= 1.0 - mountain_weight;
    }
    values[6] = mountain_weight * cold;
    values[7] = mountain_weight * (1.0 - cold);

    let sum: f64 = values.iter().sum();
    if sum > 0.0 {
        for value in values.iter_mut() {
            *value /= sum;
        }
    }
    MorphoWeights { values }
}

pub fn mix(weights: &MorphoWeights, params: &Params) -> MorphoMix {
    debug_assert!(validate(params).is_ok());
    let p = &params.morpho;
    let mut out = MorphoMix::default();
    for (k, w) in weights.values.iter().enumerate() {
        out.e_flat += w * p.e_flat[k];
        out.e_hill += w * p.e_hill[k];
        out.pv_flat += w * p.pv_flat[k];
        out.pv_hill += w * p.pv_hill[k];
        out.lambda_pv_blocks += w * p.lambda_pv_blocks[k];
        out.sigma_e += w * p.sigma_e[k];
        out.sigma_pv += w * p.sigma_pv[k];
    }
    if params.mode == Mode::Scientific {
        out.lambda_pv_blocks *= p.scientific_lambda_factor;
        out.sigma_e *= p.scientific_sigma_factor;
        out.sigma_pv *= p.scientific_sigma_factor;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{MORPHO_ZONES, mix, validate, weights};
    use crate::mode::Mode;
    use crate::params::Params;
    use crate::version::GenVersion;

    fn params() -> Params {
        Params::defaults(GenVersion::V1Dev, Mode::Gameplay)
    }

    #[test]
    fn defaults_validate_and_v0_is_rejected() {
        assert!(validate(&params()).is_ok());
        let v0 = Params::defaults(GenVersion::V0, Mode::Gameplay);
        assert!(validate(&v0).is_err());
        let mut bad = params();
        bad.morpho.t_fade_c = 10.0;
        assert!(validate(&bad).is_err());
        let mut bad = params();
        bad.morpho.lambda_pv_blocks[0] = 0.0;
        assert!(validate(&bad).is_err());
    }

    #[test]
    fn weights_form_a_partition_of_unity() {
        let p = params();
        for it in 0..24 {
            let t = -12.0 + it as f64 * 2.0;
            for ir in 0..24 {
                let r = ir as f64 * 0.25;
                for im in 0..10 {
                    let m = im as f64 / 9.0;
                    let w = weights(t, r, m, &p);
                    assert!(
                        (w.sum() - 1.0).abs() < 1e-12,
                        "sum={} t={t} r={r} m={m}",
                        w.sum()
                    );
                    for value in w.values {
                        assert!(value.is_finite() && (0.0..=1.0).contains(&value));
                    }
                }
            }
        }
    }

    #[test]
    fn endmembers_match_the_plan_table() {
        let p = params();
        let cases = [
            (-10.0, 1.0, 0.0, 0),
            (4.5, 0.8, 0.0, 1),
            (10.0, 0.6, 0.0, 2),
            (10.0, 1.5, 0.0, 3),
            (10.0, 3.0, 0.0, 4),
            (25.0, 0.2, 0.0, 5),
            (-10.0, 1.0, 1.0, 6),
            (25.0, 0.2, 1.0, 7),
        ];
        for (t, r, m, zone) in cases {
            let w = weights(t, r, m, &p);
            assert!(w.values[zone] > 0.99, "zone={zone} w={w:?}");
            assert_eq!(w.dominant(), zone);
        }
    }

    #[test]
    fn weights_are_continuous() {
        let p = params();
        let eps = 1e-6;
        for k in 0..200 {
            let t = -5.0 + k as f64 * 0.2;
            let r = 0.1 + (k % 50) as f64 * 0.08;
            let m = (k % 20) as f64 / 20.0;
            let a = weights(t, r, m, &p);
            let b = weights(t + eps, r + eps, m + eps, &p);
            for i in 0..MORPHO_ZONES {
                assert!((a.values[i] - b.values[i]).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn mix_respects_mode_and_bounds() {
        let game = params();
        let mut science = Params::defaults(GenVersion::V1Dev, Mode::Scientific);
        science.morpho = game.morpho.clone();
        let w = weights(10.0, 0.6, 0.2, &game);
        let g = mix(&w, &game);
        let s = mix(&w, &science);
        assert!(g.e_flat >= g.e_hill);
        assert!(g.pv_hill >= g.pv_flat);
        assert!(g.lambda_pv_blocks > 0.0);
        assert!(
            (s.lambda_pv_blocks - g.lambda_pv_blocks * game.morpho.scientific_lambda_factor).abs()
                < 1e-12
        );
        assert!((s.sigma_e - g.sigma_e * game.morpho.scientific_sigma_factor).abs() < 1e-12);
    }
}
