//! Жизненные зоны Холдриджа и перевод в ванильный биом (план 7.8, T40).
//! C/E/W обязаны быть непрерывными, поэтому таблица влияет только на T и H.

use crate::climate::ClimateSample;
use crate::math::{LN_2, clamp, clamp01, dexp, dln, smoothstep};
use crate::noise::{fbm2, fbm2_unit};
use crate::params::Params;
use crate::seed::{SALT_CLASS_JITTER, SALT_ECOTONE, subseed};

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

/// К2 / Д-104: кайма пустыни в `v1-dev`. Песок — ванильная ступень t4, а его
/// соседи в таблице стоят на t1…t3, поэтому пустыня набирает ступени по одной:
/// у холодного края (низ пояса П3), у порога сухости и у порога морозной зимы
/// идут сухие равнины (t2), затем саванна (t3), и только потом песок (t4).
/// Ширина одной ступени каймы по биотемпературе, °C.
const DESERT_RIM_T_C: f64 = 2.0;
/// Ширина одной ступени каймы по сухости, log2 R.
const DESERT_RIM_LOG2_R: f64 = 0.5;
/// Ширина одной ступени каймы по температуре холодного месяца, °C.
const DESERT_RIM_WINTER_C: f64 = 2.0;

fn desert_with_rim(belt: u8, t_bio: f64, ratio: f64, t_min: f64, params: &Params) -> VanillaCell {
    let c = &params.climate;
    if !c.cold_desert_sand || t_min < c.cold_desert_winter_c {
        // Морозная зима: сухие равнины, на ступень холоднее соседней по
        // сухости клетки K3 того же пояса (t2 в П3, t3 или t2 в П4, t3 выше).
        let warm = match belt {
            0..=3 => false,
            4 => t_min >= c.winter_temperate_c,
            _ => true,
        };
        return if warm {
            cell(2, 1, "plains")
        } else {
            cell(1, 0, "plains")
        };
    }
    // Каждая «высота» непрерывна по своему входу, поэтому ступень t меняется
    // по одной. На краях значения подобраны под соседей таблицы: П2 сухой —
    // t1, K3 в П3 — t2, K3 в П4+ — t3, морозная сторона — t1.
    let by_belt = if belt <= 3 {
        2.0 + (t_bio - c.belt_edges_c[2]) / DESERT_RIM_T_C
    } else {
        4.0
    };
    // Порог K3/K2: суше него таблица ставит пустыню.
    let dry_edge = dln(c.province_edges_r[3].max(1e-9)) / LN_2;
    let by_dryness = 3.0 + (dln(ratio.max(1e-9)) / LN_2 - dry_edge).max(0.0) / DESERT_RIM_LOG2_R;
    let by_winter = 2.0 + (t_min - c.cold_desert_winter_c) / DESERT_RIM_WINTER_C;
    match by_belt
        .min(by_dryness)
        .min(by_winter)
        .clamp(2.0, 4.0)
        .floor() as u8
    {
        2 => cell(2, 1, "plains"),
        3 => cell(3, 0, "savanna"),
        _ => cell(4, 0, "desert"),
    }
}

/// К2 / Д-104: шаг от ячейки `base` к ячейке `target` не больше чем на одну
/// ступень по T и по H. Так накладки (сухая котловина, галерейный лес,
/// болото) не ставят рядом ячейки через ступень.
pub(crate) fn step_toward(base: VanillaCell, target: VanillaCell) -> VanillaCell {
    let step = |from: u8, to: u8| match to.cmp(&from) {
        std::cmp::Ordering::Greater => from + 1,
        std::cmp::Ordering::Less => from - 1,
        std::cmp::Ordering::Equal => from,
    };
    let (t, h) = (step(base.t, target.t), step(base.h, target.h));
    if (t, h) == (target.t, target.h) {
        target
    } else if (t, h) == (base.t, base.h) {
        base
    } else {
        middle_cell(t, h)
    }
}

/// Таблица 7.8.2 без каймы пустыни по поясу: для контрольных точек и тестов,
/// где биотемпература внутри пояса не задана.
pub fn biome_cell(belt: u8, province: u8, ratio: f64, t_min: f64, params: &Params) -> VanillaCell {
    biome_cell_at(belt, province, f64::INFINITY, ratio, t_min, params)
}

/// Таблица 7.8.2: пояс × провинция → ванильная ячейка `(t, h)`.
/// `t_bio` нужен только кайме пустыни `v1-dev`; `v0` его не читает.
pub fn biome_cell_at(
    belt: u8,
    province: u8,
    t_bio: f64,
    ratio: f64,
    t_min: f64,
    params: &Params,
) -> VanillaCell {
    if params.version.is_water_relief_dev() {
        return biome_cell_ladder(belt, province, t_bio, ratio, t_min, params);
    }
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

/// T135 / Б6: пересмотренная таблица 7.8.2 для `v1-dev` — лестница без дыр.
///
/// Диагностика T134 показала, что скачки h≥2 на 32 блоках дают не луч влаги,
/// а сама таблица: соседние провинции/пояса в v0 отличаются на две ступени
/// (бореальный K3 h1 → K4 h3, тропический K4 h1 → K5 h3, тёплый K3 h0 → K4 h2).
/// Пятна экотона T133 такие дыры не закрывают: они дают лишь один шаг от
/// текущего класса, а через границу всё равно стоят h1 и h3.
///
/// Здесь h — 1-Липшицева функция по обеим осям таблицы: базовая лестница по
/// влажности `0,0,1,2,3,4` и потолок по температуре `1,2,3,4,4,4,4`. Соседние
/// клетки по провинции и по поясу различаются не больше чем на одну ступень,
/// поэтому переход «саванна → джунгли» всегда идёт через лес. Внутриклеточные
/// ветки v0 по R (flower_forest при R≈1, birch при R≤0.75, dark_forest при
/// R≥0.35) убраны: они создавали скачки h внутри одной провинции. Имя биома
/// берётся из средней таблицы A.2 по итовым `(t,h)`.
fn biome_cell_ladder(
    belt: u8,
    province: u8,
    t_bio: f64,
    ratio: f64,
    t_min: f64,
    params: &Params,
) -> VanillaCell {
    let belt = belt.min(6);
    let province = province.clamp(1, 6);
    // Влажность: K1/K2 сухо, дальше по одной ступени на провинцию.
    let wet = match province {
        1 | 2 => 0_u8,
        3 => 1,
        4 => 2,
        5 => 3,
        _ => 4,
    };
    // Холодные пояса не дают плотного леса: потолок растёт по одной ступени.
    let cap = match belt {
        0 => 1_u8,
        1 => 2,
        2 => 3,
        _ => 4,
    };
    let h = wet.min(cap);
    let cold = severe_winter(belt, t_min, params);
    let t = match belt {
        0 | 1 => 0_u8,
        2 => {
            if cold {
                0
            } else {
                1
            }
        }
        3 => {
            if province <= 2 {
                // Правило ⚑ холодной пустыни как в v0, плюс кайма (Д-104).
                return desert_with_rim(belt, t_bio, ratio, t_min, params);
            }
            if cold { 1 } else { 2 }
        }
        4 => {
            if province <= 2 {
                return desert_with_rim(belt, t_bio, ratio, t_min, params);
            }
            // Тёплый умеренный: сухо — саванна, влажно — лиственный лес.
            // К2 / Д-104: при суровой зиме саванны нет (t2): иначе холодный
            // П3 (t1) вставал к ней через ступень.
            if province >= 5 || t_min < params.climate.winter_temperate_c {
                2
            } else {
                3
            }
        }
        _ => {
            if province <= 2 {
                return desert_with_rim(belt, t_bio, ratio, t_min, params);
            }
            3
        }
    };
    middle_cell(t, h)
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
        cell: biome_cell_at(zone.belt, zone.province, t_bio, ratio, t_min, params),
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
    // К1 / Д-103: одно дрожание для всех версий, как в v0. Единичный fBm
    // и крупный слой T101/T133 убраны: они дробили биомы на пятна.
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
    let neighbor = biome_cell_at(
        base.zone.belt,
        neighbor_province,
        base.t_bio,
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
    fn v1_dev_table_and_desert_rim_never_jump_a_step() {
        // К2 / Д-104: по сетке непрерывных входов соседние точки дают
        // ванильные ступени T и H, отличающиеся не больше чем на одну.
        let p = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        let cell_at = |t_bio: f64, log2_r: f64, winter: f64| {
            classify_inputs(t_bio, dexp(LN_2 * log2_r), winter, &p).cell
        };
        // Каждый вход меняется отдельно: в мире два порога пересекаются
        // только в точке, и такая «диагональ» в счёт не идёт.
        let mut checked = 0_u32;
        for wi in -40..=30 {
            let winter = wi as f64;
            for ti in 0..=300 {
                let t_bio = ti as f64 * 0.1;
                for ri in -80..=80 {
                    let log2_r = ri as f64 * 0.05;
                    let here = cell_at(t_bio, log2_r, winter);
                    for next in [
                        cell_at(t_bio + 0.1, log2_r, winter),
                        cell_at(t_bio, log2_r + 0.05, winter),
                        cell_at(t_bio, log2_r, winter + 1.0),
                    ] {
                        assert!(
                            here.t.abs_diff(next.t) <= 1 && here.h.abs_diff(next.h) <= 1,
                            "T_bio {t_bio}, log2 R {log2_r}, зима {winter}: {here:?} → {next:?}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 100_000);
    }

    #[test]
    fn step_toward_moves_one_step_and_keeps_reached_target() {
        let base = cell(0, 2, "snowy_taiga");
        let stepped = step_toward(base, cell(1, 0, "plains"));
        assert_eq!((stepped.t, stepped.h), (1, 1));
        let near = step_toward(cell(3, 1, "savanna"), cell(3, 2, "forest"));
        assert_eq!(near, cell(3, 2, "forest"));
        assert_eq!(step_toward(base, base), base);
    }

    #[test]
    fn jitter_of_v1_dev_is_the_v0_jitter() {
        // К1 / Д-103: при одинаковых амплитудах дрожание v1-dev побитно
        // равно v0 (экотоны выключены, климат один и тот же).
        let old = Params::defaults(GenVersion::V0, Mode::Gameplay);
        let dev = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        assert_eq!(dev.block.jitter_t_c, old.block.jitter_t_c);
        assert_eq!(dev.block.jitter_r_log2, old.block.jitter_r_log2);
        let c = climate(15.0, 1.0);
        for k in 0..400 {
            let x = k as f64 * 37.0 - 7000.0;
            let z = k as f64 * -23.0 + 3000.0;
            let a = classify(&c, true, x, z, 42, &old);
            let b = classify(&c, true, x, z, 42, &dev);
            assert_eq!(a.t_bio.to_bits(), b.t_bio.to_bits());
            assert_eq!(a.ratio.to_bits(), b.ratio.to_bits());
        }
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

#[cfg(test)]
mod t135_tests {
    use super::*;
    use crate::mode::Mode;
    use crate::version::GenVersion;

    /// Главная причина скачков h≥2 по диагностике T134: дыры в таблице 7.8.2.
    /// Лестница v1-dev обязана быть 1-Липшицевой по провинции и по поясу.
    #[test]
    fn v1_dev_table_neighbors_differ_by_at_most_one_h_step() {
        for mode in Mode::ALL {
            let p = Params::defaults(GenVersion::V1Dev, mode);
            for t_min in [-25.0_f64, -10.0, 0.0, 8.0, 20.0] {
                for ratio in [0.2_f64, 0.45, 0.8, 1.5, 3.0, 6.0, 12.0] {
                    for belt in 0..=6_u8 {
                        for province in 1..=6_u8 {
                            let here = biome_cell(belt, province, ratio, t_min, &p);
                            if province < 6 {
                                let next = biome_cell(belt, province + 1, ratio, t_min, &p);
                                assert!(
                                    here.h.abs_diff(next.h) <= 1,
                                    "провинция {province}→{} пояс {belt} t_min {t_min}: h {}→{}",
                                    province + 1,
                                    here.h,
                                    next.h
                                );
                            }
                            if belt < 6 {
                                let next = biome_cell(belt + 1, province, ratio, t_min, &p);
                                assert!(
                                    here.h.abs_diff(next.h) <= 1,
                                    "пояс {belt}→{} провинция {province} t_min {t_min}: h {}→{}",
                                    belt + 1,
                                    here.h,
                                    next.h
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// Внутри одной провинции h не должен зависеть от R: ветки v0 по R
    /// (flower_forest/birch/dark_forest) давали скачки без смены класса.
    #[test]
    fn v1_dev_h_is_constant_inside_a_province() {
        let p = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        for belt in 0..=6_u8 {
            for province in 1..=6_u8 {
                let edge_lo = match province {
                    6 => 0.05,
                    5 => 0.51,
                    4 => 1.01,
                    3 => 2.01,
                    2 => 4.01,
                    _ => 8.01,
                };
                let edge_hi = match province {
                    6 => 0.49,
                    5 => 0.99,
                    4 => 1.99,
                    3 => 3.99,
                    2 => 7.99,
                    _ => 40.0,
                };
                let a = biome_cell(belt, province, edge_lo, 5.0, &p);
                let b = biome_cell(belt, province, edge_hi, 5.0, &p);
                assert_eq!(a.h, b.h, "пояс {belt} провинция {province}: h зависит от R");
            }
        }
    }

    #[test]
    fn v0_table_is_untouched_by_the_ladder() {
        let old = Params::defaults(GenVersion::V0, Mode::Gameplay);
        // Известные дыры v0 сохранены bitwise: бореальный K3 h1 против K4 h3.
        assert_eq!(biome_cell(2, 3, 3.0, 5.0, &old).h, 1);
        assert_eq!(biome_cell(2, 4, 1.5, 5.0, &old).h, 3);
        assert_eq!(biome_cell(6, 4, 1.5, 20.0, &old).h, 1);
        assert_eq!(biome_cell(6, 5, 0.8, 20.0, &old).h, 3);
        assert_eq!(biome_cell(3, 4, 0.9, 5.0, &old).biome, "flower_forest");
        let dev = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        assert_eq!(biome_cell(2, 3, 3.0, 5.0, &dev).h, 1);
        assert_eq!(biome_cell(2, 4, 1.5, 5.0, &dev).h, 2);
        assert_eq!(biome_cell(6, 4, 1.5, 20.0, &dev).h, 2);
        assert_eq!(biome_cell(6, 5, 0.8, 20.0, &dev).h, 3);
    }
}
