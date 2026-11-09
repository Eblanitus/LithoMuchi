//! Жизненные зоны Холдриджа и перевод в ванильный биом (план 7.8, T40).
//! C/E/W обязаны быть непрерывными, поэтому таблица влияет только на T и H.

use crate::climate::ClimateSample;
use crate::math::{LN_2, clamp, clamp01, dexp, dln, smoothstep};
use crate::noise::{fbm2, fbm2_unit};
use crate::params::Params;
use crate::seed::{SALT_CLASS_JITTER, SALT_CLASS_JITTER_COARSE, SALT_ECOTONE, subseed};

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

fn desert(t_min: f64, params: &Params) -> VanillaCell {
    // Правило ⚑ [ПО СИТУАЦИИ]: в игре нет холодной пустыни. Песок ставим
    // только там, где зима не уходит в мороз; сухие места с морозной зимой
    // (Гоби, Большой Бассейн) становятся сухими равнинами (Д-014).
    if params.climate.cold_desert_sand && t_min >= params.climate.cold_desert_winter_c {
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
        (3, 1..=2) => desert(t_min, params),
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
        (4, 1..=2) => desert(t_min, params),
        (4, 3) => cell(3, 0, "savanna"),
        (4, 4) => cell(3, 2, "forest"),
        (4, 5) => cell(2, 3, "birch_forest"),
        (4, 6) => cell(2, 4, "dark_forest"),
        (5, 1..=2) => desert(t_min, params),
        (5, 3) => cell(3, 0, "savanna"),
        (5, 4) => cell(3, 1, "savanna"),
        (5, 5) => cell(3, 2, "forest"),
        (5, 6) => cell(3, 3, "jungle"),
        (6, 1..=2) => desert(t_min, params),
        (6, 3) => cell(3, 0, "savanna"),
        (6, 4) => cell(3, 1, "savanna"),
        (6, 5) => cell(3, 3, "jungle"),
        (6, 6) => cell(3, 4, "jungle"),
        _ => desert(t_min, params),
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
    let (jitter_t, jitter_r) = if params.version.is_water_relief_dev() {
        // T101/T133: амплитуды в σ. Мелкий масштаб λ240 остаётся прежним;
        // крупный λ1000 добавляет независимое дрожание с σ в factor раз больше
        // (стартовый factor 4 ≈ смещение границ ±200 против ±50 блоков).
        let b = &params.boundaries;
        let coarse = subseed(seed, SALT_CLASS_JITTER_COARSE);
        let cu = x / b.jitter_coarse_lambda_blocks;
        let cv = z / b.jitter_coarse_lambda_blocks;
        let f = b.jitter_coarse_sigma_factor;
        (
            fbm2_unit(sub, u, v, 1.0, c.jitter_octaves, c.jitter_gain)
                + f * fbm2_unit(coarse, cu, cv, 1.0, c.jitter_octaves, c.jitter_gain),
            fbm2_unit(
                sub.wrapping_add(1),
                u,
                v,
                1.0,
                c.jitter_octaves,
                c.jitter_gain,
            ) + f * fbm2_unit(
                coarse.wrapping_add(1),
                cu,
                cv,
                1.0,
                c.jitter_octaves,
                c.jitter_gain,
            ),
        )
    } else {
        (
            fbm2(sub, u, v, 1.0, c.jitter_octaves, c.jitter_gain),
            fbm2(
                sub.wrapping_add(1),
                u,
                v,
                1.0,
                c.jitter_octaves,
                c.jitter_gain,
            ),
        )
    };
    let t_bio = climate.temperature.t_bio + params.block.jitter_t_c * jitter_t;
    let log2_r = clamp(
        dln(climate.precip.ratio.max(1e-9)) / LN_2 + params.block.jitter_r_log2 * jitter_r,
        -10.0,
        10.0,
    );
    let ratio = dexp(LN_2 * log2_r);
    let mut result = classify_inputs(t_bio, ratio, climate.temperature.t_min, params);
    if params.version.is_water_relief_dev() && !is_ocean {
        result.cell = ecotone_cell(
            result,
            log2_r,
            climate.temperature.t_min,
            x,
            z,
            seed,
            params,
        );
    }
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

/// T133 / Б6: у порога влажности часть пятен получает промежуточную ячейку
/// соседней провинции. Это не сглаживание C/E/W и не новый биом: только
/// ванильная ячейка `(t,h)` между сухим и влажным классом, чтобы саванна не
/// стояла вплотную к джунглям. Решение гладкое по координатам, пятно λ≥64.
fn ecotone_cell(
    base: ClassSample,
    log2_r: f64,
    t_min: f64,
    x: f64,
    z: f64,
    seed: u64,
    params: &Params,
) -> VanillaCell {
    let b = &params.boundaries;
    if b.ecotone_width_log2 <= 0.0 || b.ecotone_max_fraction <= 0.0 {
        return base.cell;
    }
    let edges = params.climate.province_edges_r;
    let mut best = f64::INFINITY;
    let mut neighbor_province = base.zone.province;
    for (k, edge) in edges.iter().enumerate() {
        let edge_log2 = dln(*edge) / LN_2;
        let dist = (log2_r - edge_log2).abs();
        if dist < best {
            best = dist;
            // Провинции нумеруются от влажной K6 к сухой K1; переход через
            // порог вверх по R уменьшает номер провинции на единицу.
            neighbor_province = if log2_r < edge_log2 {
                5_u8.saturating_sub(k as u8)
            } else {
                6_u8.saturating_sub(k as u8)
            }
            .clamp(1, 6);
        }
    }
    if !best.is_finite() || best >= b.ecotone_width_log2 || neighbor_province == base.zone.province
    {
        return base.cell;
    }
    let fraction = b.ecotone_max_fraction * (1.0 - smoothstep(0.0, b.ecotone_width_log2, best));
    let n = fbm2_unit(
        subseed(seed, SALT_ECOTONE),
        x / b.ecotone_lambda_blocks,
        z / b.ecotone_lambda_blocks,
        1.0,
        1,
        0.5,
    );
    if clamp01(0.5 + 0.5 * n) >= fraction {
        return base.cell;
    }
    let neighbor_ratio = dexp(
        LN_2 * (log2_r
            + if neighbor_province < base.zone.province {
                0.5
            } else {
                -0.5
            }),
    );
    let neighbor = biome_cell(
        base.zone.belt,
        neighbor_province,
        neighbor_ratio,
        t_min,
        params,
    );
    let dh = neighbor.h as i8 - base.cell.h as i8;
    if dh == 0 {
        return base.cell;
    }
    // Лестница: пятно делает один шаг от текущего класса к соседнему, а не
    // повторяет весь скачок h0→h3. Имя берётся из средней таблицы A.2.
    let h = (base.cell.h as i8 + dh.signum()).clamp(0, 4) as u8;
    middle_cell(base.cell.t, h)
}

fn middle_cell(t: u8, h: u8) -> VanillaCell {
    let name = match (t.min(4), h.min(4)) {
        (0, 0 | 1) => "snowy_plains",
        (0, 2) => "snowy_taiga",
        (0, 3) => "snowy_taiga",
        (0, _) => "taiga",
        (1, 0 | 1) => "plains",
        (1, 2) => "forest",
        (1, 3) => "taiga",
        (1, _) => "old_growth_spruce_taiga",
        (2, 0) => "flower_forest",
        (2, 1) => "plains",
        (2, 2) => "forest",
        (2, 3) => "birch_forest",
        (2, _) => "dark_forest",
        (3, 0 | 1) => "savanna",
        (3, 2) => "forest",
        (3, _) => "jungle",
        _ => "desert",
    };
    cell(t.min(4), h.min(4), name)
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
    fn dry_places_with_freezing_winters_are_plains_not_sand() {
        let p = params();
        assert_eq!(biome_cell(3, 1, 20.0, -7.0, &p).biome, "plains");
        assert_eq!(biome_cell(3, 1, 20.0, 5.0, &p).biome, "desert");
        assert_eq!(biome_cell(5, 1, 20.0, 15.0, &p).biome, "desert");
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

#[cfg(test)]
mod t133_tests {
    use super::*;
    use crate::climate::{ClimateSample, PrecipSample, SurfaceSample, TemperatureSample};
    use crate::math::Vec2;
    use crate::mode::Mode;
    use crate::version::GenVersion;

    fn climate(t_bio: f64, ratio: f64) -> ClimateSample {
        ClimateSample {
            point: Vec2::default(),
            surface: SurfaceSample::default(),
            pole: crate::climate::PoleSample {
                phi_deg: 45.0,
                north: Vec2::new(0.0, 1.0),
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
                t_min: 5.0,
            },
            precip: PrecipSample {
                mu: 0.5,
                cont: 0.5,
                p_mm: 1000.0,
                pet_mm: 1000.0 * ratio,
                ratio,
                slope: 0.0,
            },
        }
    }

    #[test]
    fn coarse_jitter_moves_classes_more_than_fine_and_v0_is_bitwise_unchanged() {
        let dev = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        let mut fine_only = dev.clone();
        fine_only.boundaries.jitter_coarse_sigma_factor = 1.0;
        let old = Params::defaults(GenVersion::V0, Mode::Gameplay);
        let mut changed_old = old.clone();
        changed_old.boundaries.jitter_coarse_sigma_factor = 8.0;
        changed_old.boundaries.ecotone_max_fraction = 0.5;
        let c = climate(15.0, 1.0);
        let mut dev_moves = 0;
        let mut fine_moves = 0;
        for k in 0..400 {
            let x = k as f64 * 37.0 - 7000.0;
            let z = k as f64 * -23.0 + 3000.0;
            let base = classify_inputs(15.0, 1.0, 5.0, &dev);
            let d = classify(&c, false, x, z, 42, &dev);
            let f = classify(&c, false, x, z, 42, &fine_only);
            dev_moves += usize::from(d.cell != base.cell);
            fine_moves += usize::from(f.cell != base.cell);
            let a = classify(&c, false, x, z, 42, &old);
            let b = classify(&c, false, x, z, 42, &changed_old);
            assert_eq!(a.cell, b.cell);
            assert_eq!(a.t_bio.to_bits(), b.t_bio.to_bits());
            assert_eq!(a.ratio.to_bits(), b.ratio.to_bits());
        }
        assert!(
            dev_moves > fine_moves,
            "крупный масштаб должен двигать границы сильнее: {dev_moves} vs {fine_moves}"
        );
    }

    #[test]
    fn ecotone_is_one_step_only_near_humidity_thresholds_and_never_in_ocean() {
        let mut p = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        p.boundaries.ecotone_max_fraction = 0.5;
        p.boundaries.ecotone_width_log2 = 0.5;
        let mut no_eco = p.clone();
        no_eco.boundaries.ecotone_max_fraction = 0.0;
        let mut seen = 0;
        for k in 0..2000 {
            let x = k as f64 * 17.0 - 17000.0;
            let z = k as f64 * 11.0 - 9000.0;
            // Точно на пороге R=2 (K4/K5) и вдали от него.
            for ratio in [2.0, 1000.0] {
                let c = climate(27.0, ratio);
                let base = classify(&c, false, x, z, 42, &no_eco);
                let got = classify(&c, false, x, z, 42, &p);
                let ocean = classify(&c, true, x, z, 42, &p);
                assert!(ocean.is_ocean);
                assert_eq!(ocean.cell, ocean_cell(c.temperature.sst_c, &p));
                let dh = (got.cell.h as i8 - base.cell.h as i8).abs();
                assert!(
                    dh <= 1,
                    "экотон не должен повторять скачок h0→h3: {base:?} → {got:?}"
                );
                if ratio == 1000.0 {
                    assert_eq!(got.cell, base.cell, "вдали от порога экотона нет");
                } else if got.cell != base.cell {
                    seen += 1;
                    assert_eq!(got.cell.t, base.cell.t);
                }
            }
        }
        assert!(seen > 0, "мозаика экотона должна встречаться у порога");
    }
}
