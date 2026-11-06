//! Жизненные зоны Холдриджа и перевод в ванильный биом (план 7.8, T40).
//! C/E/W обязаны быть непрерывными, поэтому таблица влияет только на T и H.

use crate::climate::ClimateSample;
use crate::math::{LN_2, clamp, dexp, dln};
use crate::noise::fbm2;
use crate::params::Params;
use crate::seed::{SALT_CLASS_JITTER, subseed};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LifeZone {
    /// П0…П6 по биотемпературе.
    pub belt: u8,
    /// К1…К6 по индексу сухости (1 — пустыня, 6 — пергумидная).
    pub province: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VanillaCell {
    /// Индекс ванильной полосы температуры 0…4.
    pub t: u8,
    /// Индекс ванильной полосы влажности 0…4.
    pub h: u8,
    pub biome: &'static str,
}

const fn cell(t: u8, h: u8, biome: &'static str) -> VanillaCell {
    VanillaCell { t, h, biome }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClassSample {
    pub zone: LifeZone,
    pub cell: VanillaCell,
    pub is_ocean: bool,
    pub t_bio: f64,
    pub ratio: f64,
}

fn belt_of(t_bio: f64, params: &Params) -> u8 {
    let mut belt = 0_u8;
    for edge in params.climate.belt_edges_c {
        if t_bio >= edge {
            belt += 1;
        } else {
            break;
        }
    }
    belt
}

fn province_of(ratio: f64, params: &Params) -> u8 {
    let mut province = 1_u8;
    for (k, edge) in params.climate.province_edges_r.iter().enumerate() {
        if ratio < *edge {
            province = 6_u8 - k as u8;
            break;
        }
    }
    province
}

fn desert(params: &Params) -> VanillaCell {
    // Правило ⚑ «холодные пустыни» [ПО СИТУАЦИИ]: по умолчанию песчаная
    // пустыня, альтернатива — равнины. Переключается до оценки автором (В2).
    if params.climate.cold_desert_sand {
        cell(4, 0, "desert")
    } else {
        cell(1, 0, "plains")
    }
}

fn severe_winter(belt: u8, t_min: f64, params: &Params) -> bool {
    match belt {
        2 => t_min < params.climate.winter_boreal_c,
        3 => t_min < params.climate.winter_temperate_c,
        _ => false,
    }
}

/// Таблица 7.8.2: пояс × провинция → ванильная ячейка `(t, h)`.
pub fn biome_cell(belt: u8, province: u8, ratio: f64, t_min: f64, params: &Params) -> VanillaCell {
    let cold = severe_winter(belt, t_min, params);
    match (belt, province) {
        (0, _) => cell(0, 0, "snowy_plains"),
        (1, 1..=3) => cell(0, 1, "snowy_plains"),
        (1, 4..=6) => cell(0, 2, "snowy_taiga"),
        (2, 1..=2) => cell(1, 0, "plains"),
        (2, 3) => cell(1, 1, "plains"),
        (2, 4..=5) => {
            if cold {
                cell(0, 3, "snowy_taiga")
            } else {
                cell(1, 3, "taiga")
            }
        }
        (2, 6) => {
            if cold {
                cell(0, 4, "taiga")
            } else {
                cell(1, 4, "old_growth_spruce_taiga")
            }
        }
        (3, 1..=2) => desert(params),
        (3, 3) => {
            if cold {
                cell(1, 1, "plains")
            } else {
                cell(2, 1, "plains")
            }
        }
        (3, 4) => {
            if (0.75..=1.0).contains(&ratio) {
                cell(2, 0, "flower_forest")
            } else if cold {
                cell(1, 1, "plains")
            } else {
                cell(2, 1, "plains")
            }
        }
        (3, 5) => {
            if ratio > 0.75 {
                cell(2, 2, "forest")
            } else if cold {
                cell(1, 3, "taiga")
            } else {
                cell(2, 3, "birch_forest")
            }
        }
        (3, 6) => {
            if ratio >= 0.35 {
                cell(2, 4, "dark_forest")
            } else {
                cell(1, 4, "old_growth_spruce_taiga")
            }
        }
        (4, 1..=2) => desert(params),
        (4, 3) => cell(3, 0, "savanna"),
        (4, 4) => cell(3, 2, "forest"),
        (4, 5) => cell(2, 3, "birch_forest"),
        (4, 6) => cell(2, 4, "dark_forest"),
        (5, 1..=2) => desert(params),
        (5, 3) => cell(3, 0, "savanna"),
        (5, 4) => cell(3, 1, "savanna"),
        (5, 5) => cell(3, 2, "forest"),
        (5, 6) => cell(3, 3, "jungle"),
        (6, 1..=2) => desert(params),
        (6, 3) => cell(3, 0, "savanna"),
        (6, 4) => cell(3, 1, "savanna"),
        (6, 5) => cell(3, 3, "jungle"),
        (6, 6) => cell(3, 4, "jungle"),
        _ => desert(params),
    }
}

/// Океаны по SST (7.8): категория решает только ванильная полоса температуры.
pub fn ocean_cell(sst_c: f64, params: &Params) -> VanillaCell {
    let bands = params.climate.sst_bands_c;
    if sst_c < bands[0] {
        cell(0, 0, "frozen_ocean")
    } else if sst_c < bands[1] {
        cell(1, 0, "cold_ocean")
    } else if sst_c < bands[2] {
        cell(2, 0, "ocean")
    } else if sst_c < bands[3] {
        cell(3, 0, "lukewarm_ocean")
    } else {
        cell(4, 0, "warm_ocean")
    }
}

/// Классификация на прямых входах, без дрожания границ (контрольные точки).
pub fn classify_inputs(t_bio: f64, ratio: f64, t_min: f64, params: &Params) -> ClassSample {
    let zone = LifeZone {
        belt: belt_of(t_bio, params),
        province: province_of(ratio, params),
    };
    ClassSample {
        zone,
        cell: biome_cell(zone.belt, zone.province, ratio, t_min, params),
        is_ocean: false,
        t_bio,
        ratio,
    }
}

/// Классификация в точке мира: добавляет «дрожание» границ шумом в блоках
/// (7.8.1) и выбирает сухопутную или морскую таблицу по знаку C (7.9.4).
pub fn classify(
    climate: &ClimateSample,
    is_ocean: bool,
    x: f64,
    z: f64,
    seed: u64,
    params: &Params,
) -> ClassSample {
    let c = &params.climate;
    let sub = subseed(seed, SALT_CLASS_JITTER);
    let u = x / c.jitter_lambda_blocks;
    let v = z / c.jitter_lambda_blocks;
    let jitter_t = fbm2(sub, u, v, 1.0, c.jitter_octaves, c.jitter_gain);
    let jitter_r = fbm2(
        sub.wrapping_add(1),
        u,
        v,
        1.0,
        c.jitter_octaves,
        c.jitter_gain,
    );
    let t_bio = climate.temperature.t_bio + params.block.jitter_t_c * jitter_t;
    let log2_r = clamp(
        dln(climate.precip.ratio.max(1e-9)) / LN_2 + params.block.jitter_r_log2 * jitter_r,
        -10.0,
        10.0,
    );
    let ratio = dexp(LN_2 * log2_r);
    let mut result = classify_inputs(t_bio, ratio, climate.temperature.t_min, params);
    if is_ocean {
        result.cell = ocean_cell(climate.temperature.sst_c, params);
        result.zone = LifeZone {
            belt: result.cell.t,
            province: 1,
        };
        result.is_ocean = true;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{biome_cell, classify_inputs, ocean_cell};
    use crate::mode::Mode;
    use crate::params::Params;
    use crate::version::GenVersion;

    fn params() -> Params {
        Params::defaults(GenVersion::V0, Mode::Gameplay)
    }

    #[test]
    fn every_table_cell_is_a_valid_vanilla_cell() {
        let p = params();
        for belt in 0..=6_u8 {
            for province in 1..=6_u8 {
                for ratio in [0.1_f64, 0.6, 1.5, 3.0, 12.0] {
                    for t_min in [-30.0_f64, 5.0] {
                        let c = biome_cell(belt, province, ratio, t_min, &p);
                        assert!(c.t <= 4, "belt={belt} province={province}: t={}", c.t);
                        assert!(c.h <= 4, "belt={belt} province={province}: h={}", c.h);
                        assert!(!c.biome.is_empty());
                    }
                }
            }
        }
    }

    #[test]
    fn cold_winter_moves_boreal_and_temperate_biomes_one_row_colder() {
        let p = params();
        let warm = biome_cell(2, 5, 0.7, 0.0, &p);
        let cold = biome_cell(2, 5, 0.7, -20.0, &p);
        assert_eq!(warm.biome, "taiga");
        assert_eq!(cold.biome, "snowy_taiga");
        assert_eq!(cold.t, 0);
    }

    #[test]
    fn ocean_bands_are_monotonic() {
        let p = params();
        let names: Vec<&str> = [-5.0, 5.0, 15.0, 20.0, 27.0]
            .iter()
            .map(|sst| ocean_cell(*sst, &p).biome)
            .collect();
        assert_eq!(
            names,
            [
                "frozen_ocean",
                "cold_ocean",
                "ocean",
                "lukewarm_ocean",
                "warm_ocean"
            ]
        );
    }

    #[test]
    fn dryness_and_temperature_select_expected_belts() {
        let p = params();
        assert_eq!(classify_inputs(0.5, 20.0, -20.0, &p).zone.belt, 0);
        assert_eq!(classify_inputs(10.0, 0.6, 0.0, &p).zone.belt, 3);
        assert_eq!(classify_inputs(27.0, 0.6, 20.0, &p).zone.belt, 6);
        assert_eq!(classify_inputs(27.0, 20.0, 20.0, &p).zone.province, 1);
        assert_eq!(classify_inputs(27.0, 0.2, 20.0, &p).zone.province, 6);
    }
}
