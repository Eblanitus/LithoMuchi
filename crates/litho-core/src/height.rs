//! T144: собственная функция высоты (полная I2, PLAN-REVISION §6.7).
//!
//! Это не ванильные сплайны Mojang и не их копия: своя таблица высот по
//! полосам E и PV, свой ocean/coast переход, factor/jaggedness по типу
//! рельефа. В научном режиме поверх таблицы идёт реальная макровысота `e_km`.
//! Вырез русел/котловин под зеркало воды добавляет T145; здесь считается
//! крупная форма поверхности, которую предпросмотр может показать точно.

use crate::hydrology::HydrologySample;
use crate::math::{clamp, clamp01, lerp, piecewise, smoothstep};
use crate::params::Params;

/// Уровень моря в блоках. Вертикаль Minecraft: 1 блок ≈ 1 м.
pub const SEA_LEVEL: f64 = 63.0;
/// База формулы offset игры: поверхность y = 128·(1 + offset), то есть
/// offset = y/128 − 1. В ванили внутри offset сидит константа −0,50375,
/// дающая уровень 63,52; пак заменяет offset целиком, поэтому константа
/// должна быть в нашей формуле, а не в паке (К2, шаг 2 плана).
pub const SURFACE_BASE: f64 = 128.0;
/// R2: насколько глубже сухой поверхности русло может прорезать бугор дна
/// долины, блоки. Больше — уже не река, а каньон К3: колонка остаётся сушей.
pub const RIVER_GORGE_MAX_BLOCKS: f64 = 8.0;
/// R2: насколько уровень русла может стоять выше сухой поверхности колонки,
/// блоки. Выше — это водопад, а не русло.
pub const RIVER_MAX_ABOVE_LAND_BLOCKS: f64 = 3.0;
pub const OFFSET_SCALE: f64 = 128.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeightSample {
    /// Плотность `overworld/offset`: surface_y/128 − 1. В игре глубина
    /// на высоте y равна (1 − y/128) + offset, поэтому поверхность
    /// получается ровно на surface_y.
    pub offset: f64,
    pub factor: f64,
    pub jaggedness: f64,
    /// Крупная форма поверхности в блоках, до ванильной 3D-фактуры/пещер.
    /// T145: после with_water это дно русла/котловины, вырезанное под зеркало.
    pub surface_y: f64,
    /// T145: зеркало воды в блоках; вне рек/озёр — уровень моря.
    pub water_y: f64,
    /// T145: дно под зеркалом (mirror - depth), до смешивания с берегом.
    pub bed_y: f64,
    /// T145: сила выреза русла/озера, 0…1.
    pub water_strength: f64,
    /// ВО2-1 (PLAN-WATER-UPDATE §6.7): маска `calm` — ноль в воде, на берегу,
    /// дне долины и озере (плюс отступ), снаружи затухает smoothstep-ом.
    /// Для версий, кроме `v1-dev-w`, ровно 1.0.
    pub calm: f64,
}

impl Default for HeightSample {
    fn default() -> Self {
        Self {
            offset: (SEA_LEVEL - SURFACE_BASE) / OFFSET_SCALE,
            factor: 0.0,
            jaggedness: 0.0,
            surface_y: SEA_LEVEL,
            water_y: SEA_LEVEL,
            bed_y: SEA_LEVEL,
            water_strength: 0.0,
            calm: 1.0,
        }
    }
}

/// Непрерывное значение по полосам E. Узлы — границы полос_stats; на границе
/// берём среднее соседних полос, чтобы таблица не давала обрыв на стыке E.
fn by_erosion(e: f64, rows: &[f64; 7], knots: &[f64; 8]) -> f64 {
    let mut values = [0.0; 8];
    values[0] = rows[0];
    values[7] = rows[6];
    for i in 1..7 {
        values[i] = 0.5 * (rows[i - 1] + rows[i]);
    }
    piecewise(knots, &values, e)
}

/// T144: высота/factor/jaggedness из тех же C/E/PV, что выбирают биом.
/// `cold`/`warm` — веса холодной/тёплой горной морфозоны T121; они делают
/// холодные пики острее, не переключая тип рельефа скачком.
pub fn sample(
    c: f64,
    e: f64,
    pv: f64,
    e_km: f64,
    cold: f64,
    warm: f64,
    params: &Params,
) -> HeightSample {
    // Без данных о береге (песочница/тесты): спуск К4 не применяется.
    sample_micro(c, e, pv, e_km, cold, warm, 0.0, 0.0, f64::INFINITY, params)
}

/// Д-057: то же с мелким рельефом. `noise` — единичный шум бугров, `gate` —
/// его вес 0…1 (в долине реки 0, чтобы дно оставалось на уровне русла).
/// Размах в блоках берётся по полосе E. Ниже `micro_low_y` бугры только
/// поднимают поверхность, поэтому низменность не затапливается.
/// К4: `shore_dist` — расстояние до берега океана в блоках (генератор
/// оценивает его по градиенту континентальности); `INFINITY` — берега нет.
#[allow(clippy::too_many_arguments)]
pub fn sample_micro(
    c: f64,
    e: f64,
    pv: f64,
    e_km: f64,
    cold: f64,
    warm: f64,
    noise: f64,
    gate: f64,
    shore_dist: f64,
    params: &Params,
) -> HeightSample {
    debug_assert!(validate(params).is_ok());
    let h = &params.height;
    let pv = clamp(pv, h.pv_knots[0], h.pv_knots[h.pv_knots.len() - 1]);
    let mut rows = [0.0; 7];
    for (k, row) in h.land_y.iter().enumerate() {
        rows[k] = piecewise(&h.pv_knots, row, pv);
    }
    let mut land_y = by_erosion(e, &rows, &h.e_knots);
    let gate = clamp01(gate);
    if gate > 0.0 && noise.is_finite() {
        let amp = by_erosion(e, &h.micro_amp_blocks, &h.e_knots);
        // Гладкая «только вверх»: (√(n²+1)+n)/2 ≥ 0 и → max(n, 0) при |n|≫1.
        let up = 0.5 * ((noise * noise + 1.0).sqrt() + noise);
        let high = smoothstep(h.micro_low_y, h.micro_high_y, land_y);
        land_y += gate * amp * lerp(up, noise, high);
    }
    let ocean_y = piecewise(&h.ocean_c, &h.ocean_y, c);
    let threshold = params.climate.ocean_c_threshold;
    let coast_c = smoothstep(threshold - h.coast_width_c, threshold + h.coast_width_c, c);
    // К4: суша спускается к воде на `shore_width_blocks` от берега по
    // расстоянию, а не только по окну C — окно сжимается на крутом шельфе.
    // Горный берег (низкая эрозия) обрыв сохраняет, как каменный в ванили.
    let ramp = if e < h.shore_cliff_e || h.shore_width_blocks <= 0.0 {
        1.0
    } else {
        smoothstep(0.0, h.shore_width_blocks, shore_dist)
    };
    let coast = coast_c.min(ramp);
    let mut y = ocean_y + (land_y - ocean_y) * coast;
    // Научный режим: реальная макровысота поверх своей таблицы. Gameplay
    // остаётся на блочной таблице; e_gain по умолчанию 0.
    // Д-057: ниже `e_gain_base_km` суша остаётся на таблице (низменности не
    // всплывают), выше прибавляется только превышение. Океан (e_km<0) — как был.
    let excess_km = if e_km > h.e_gain_base_km {
        e_km - h.e_gain_base_km
    } else {
        e_km.min(0.0)
    };
    y += h.e_gain_blocks_per_km * excess_km;
    let y = clamp(y, h.min_y, h.max_y);
    // Шкала ванили: factor ~0,6…6,3 (равнины ~5–6, горы ~1), jaggedness 0…1
    // (0 вне холодных пиков). На первый тест пак их не переопределяет —
    // ваниль посчитает сама из C/E/PV; таблица здесь для будущего возврата.
    let factor = clamp(by_erosion(e, &h.factor, &h.e_knots), 0.5, 6.5);
    let jaggedness = clamp(
        by_erosion(e, &h.jaggedness, &h.e_knots) + h.cold_jagged_gain * cold
            - h.warm_jagged_drop * warm,
        0.0,
        1.0,
    );
    HeightSample {
        offset: (y - SURFACE_BASE) / OFFSET_SCALE,
        factor,
        jaggedness,
        surface_y: y,
        water_y: SEA_LEVEL,
        bed_y: y,
        water_strength: 0.0,
        calm: 1.0,
    }
}

/// T145: вырез русел/котловин под зеркало воды (§5.9). Река берёт монотонное
/// зеркало T145 из ChannelHit и не поднимается выше локального берега −1.
/// Озеро стоит плоским зеркалом на уровне перелива; дно = зеркало − глубина.
/// Это своя высота для предпросмотра; игра ставит воду через T143.
///
/// R2 (`v1-dev-r2`, PLAN-WATER-FABRIC §4): уровень берётся от игровой
/// поверхности, а вырез рельеф только опускает. Предел
/// `hydrology.r1_max_level_km` снимается, дно гарантированно ниже зеркала,
/// и только нехватку ≤ `height.bank_raise_max_blocks` добивает берег.
pub fn with_water(
    mut height: HeightSample,
    hydrology: &HydrologySample,
    params: &Params,
) -> HeightSample {
    let h = &params.height;
    let land_y = height.surface_y;
    let r2 = params.version.is_water_relief_r2();
    // R2: все озёра с водой режутся и наливаются, `DeferredWater` не нужен.
    let r1_limit_km = if r2 {
        f64::INFINITY
    } else {
        params.hydrology.r1_max_level_km
    };
    let mut best = 0.0_f64;
    let mut water_y = SEA_LEVEL;
    let mut bed_y = land_y;
    // Поверхность после выреза у выбранной воды: у реки вырез ограничен
    // глубиной русла, у озера — нет (котловина и есть вырез).
    let mut carve_y: Option<f64> = None;
    if let Some(hit) = hydrology.channels.nearest {
        // Д-057: вода и вырез — по узкому берегу русла, а не по широкому
        // переходу C/E/W. Долину вокруг даёт PV→−1 и таблица высот.
        // R2 (§4, пп. 2–3): вода стоит только в русле шириной `core`. Уровень
        // берётся с `core/2 + 1`, поэтому кромка воды и проба берега — соседи
        // через блок; если лить по широкой полосе `bank`, кромка уезжает на
        // 8 блоков и зеркало оказывается выше суши у кромки (протечка п. 2).
        let cut = if r2 {
            clamp01(hydrology.channels.core)
        } else {
            clamp01(hydrology.channels.bank)
        };
        // R2 (§4, п. 2): уровень вершины русла — низший берег минус 1,
        // монотонно вниз по течению.
        //
        // Нет уровня R2 (берег не выше 64, вода не выше моря) — русло всё
        // равно режется, как в v1-dev, под зеркало `min(63, S − 1)`: воду
        // до y = 62 ставит сама игра. Дно речных долин по таблице высот
        // стоит на 63–65, поэтому без этого выреза все реки R2 оставались
        // сухими на 63,5 с лужами там, где дно опустил 3D-шум (игровой тест
        // 2026-11-19: 0 % мокрых осей русла против 66 % / 84 % в v1-dev).
        // Уровень здесь не выше моря, так что `water_column` R2-воды не
        // отдаёт, и инвариант §4 не затрагивается.
        //
        // Уровень R2 выше сухой поверхности больше чем на
        // `RIVER_MAX_ABOVE_LAND_BLOCKS` — не русло, а водопад: уровень сегмента
        // интерполируется между вершинами, и там, где рельеф круто падает
        // (сид 42, (-6497, -2222) и (-8418, 1772): зеркало 74 над сушей 60),
        // вода висела пластом над пустотой. Такая колонка берёт запасной
        // уровень у моря.
        let depth = hit.depth_blocks.max(1.0);
        let r2_level = hit
            .level_blocks
            .filter(|level| *level <= land_y + RIVER_MAX_ABOVE_LAND_BLOCKS);
        let mirror = if r2 {
            r2_level.or(Some(SEA_LEVEL.min(land_y - 1.0)))
        } else {
            Some(hit.mirror_y.round().min(land_y - 1.0))
        };
        // Ущелье только у русла с уровнем R2; русло у моря режется на глубину
        // русла плюс блок, иначе на суше 70 выкапывалась бы траншея до 61.
        let gorge = if r2_level.is_some() {
            RIVER_GORGE_MAX_BLOCKS.max(depth)
        } else {
            depth + 1.0
        };
        if let Some(mirror) = mirror.filter(|mirror| *mirror >= h.min_y) {
            if cut > 0.0 {
                let bed = (mirror - depth).max(h.min_y);
                let carve = if r2 {
                    // §4, п. 3: вырез не глубже `RIVER_GORGE_MAX_BLOCKS` под
                    // сушу. Иначе зеркало с километровым уровнем выкапывало бы
                    // каньон в 30 блоков (баг К3).
                    //
                    // Раньше предел был глубиной русла (1,5–2 блока). Но уровень
                    // реки — низший берег, только вниз по течению, а дно долины
                    // по таблице высот идёт буграми ±5 блоков. Вода стояла лишь
                    // в ямах между буграми: в окнах water-leaks 60–83 % осей
                    // русла выше моря были сухими (игровой тест 2026-11-19,
                    // «реки лужами»). Теперь русло прорезает бугор ущельем до
                    // `RIVER_GORGE_MAX_BLOCKS`, как настоящая река.
                    lerp(land_y, bed, cut).max(land_y - gorge).min(land_y)
                } else {
                    lerp(land_y, bed, cut)
                };
                // Вода стоит только там, где вырез опустил дно ниже зеркала.
                let wet = !r2 || carve < mirror - 0.5;
                if cut > best && wet {
                    best = cut;
                    water_y = mirror;
                    bed_y = bed;
                    carve_y = Some(carve);
                }
            }
        }
    }
    if let Some(hit) = hydrology.lakes.water_hit.filter(|hit| hit.inside) {
        // R2 (§4, п. 1): плоское зеркало = min(S) по контуру минус 1, блоки.
        // `level_km` модели нужен только для решения о воде и для контура.
        if r2 {
            if let Some(mirror) = hit.level_blocks.filter(|level| *level >= h.min_y) {
                let cut = clamp01(hit.water_strength);
                // Колонка выше зеркала — остров или берег: её не режем.
                if cut > 0.0 && land_y < mirror {
                    let depth = clamp(
                        hit.max_depth_km * h.water_km_to_blocks,
                        1.0,
                        h.lake_depth_max_blocks,
                    );
                    let bed = (mirror - depth).max(h.min_y);
                    let carve = lerp(land_y, bed, cut).min(land_y);
                    if (cut > best || (cut == best && bed < bed_y)) && carve < mirror - 0.5 {
                        best = cut;
                        water_y = mirror;
                        bed_y = bed;
                        carve_y = Some(carve);
                    }
                }
            }
        } else if let Some(level) = hit.level_km {
            // К3: до R2 вода выше R1 отложена (DeferredWater) — рельеф не
            // вырезать и не поднимать, котловина остаётся сухой сушёй.
            if level <= r1_limit_km {
                let cut = clamp01(hit.water_strength);
                if cut > 0.0 {
                    // К3: в R1 зеркало не выше локального берега, как у рек, —
                    // дно не может встать плоским столом над сушей.
                    let raw = (SEA_LEVEL + level.max(0.0) * h.water_km_to_blocks).round();
                    let mirror = raw.min(land_y - 1.0);
                    if mirror >= h.min_y {
                        let depth = clamp(
                            hit.max_depth_km * h.water_km_to_blocks,
                            1.0,
                            h.lake_depth_max_blocks,
                        );
                        let bed = (mirror - depth).max(h.min_y);
                        let carve = lerp(land_y, bed, cut);
                        if cut > best || (cut == best && bed < bed_y) {
                            best = cut;
                            water_y = mirror;
                            bed_y = bed;
                            carve_y = Some(carve);
                        }
                    }
                }
            }
        }
    }
    if let Some(carve) = carve_y.filter(|_| best > 0.0) {
        // §4, пп. 1 и 3: вырез только вниз (`min(S, вырез)`), дно ниже зеркала.
        height.surface_y = if r2 {
            carve.min(land_y).min(water_y - 1.0)
        } else {
            carve
        };
        height.water_y = water_y;
        height.bed_y = bed_y;
        height.water_strength = best;
        height.offset = (height.surface_y - SURFACE_BASE) / OFFSET_SCALE;
    }
    if r2 {
        height = raise_bank(height, hydrology, params, best);
    }
    height
}

/// R2 (PLAN-WATER-FABRIC §4, п. 4): берег — только добивка.
///
/// Уровень воды считается от сухой поверхности `S`, поэтому вода и суша
/// почти всегда сходятся сами. Поднимать берег нужно ровно там, где до
/// зеркала не хватает одного-двух блоков (`height.bank_raise_max_blocks`):
/// у самой кромки суша встаёт ровно на уровень воды, без плавного спада.
/// Нехватка больше — это ошибка уровня (пп. 1–2), а не берега, и такой
/// берег не трогаем: иначе возвращаются насыпи и озёра-столы.
fn raise_bank(
    mut height: HeightSample,
    hydrology: &HydrologySample,
    params: &Params,
    cut: f64,
) -> HeightSample {
    let h = &params.height;
    if cut > 0.0 || h.bank_raise_blocks < 0.0 || h.bank_raise_max_blocks <= 0.0 {
        return height;
    }
    let land_y = height.surface_y;
    let l = params.block.l_blocks;
    let mut target = land_y;
    let mut consider = |level: f64, distance_blocks: f64| {
        if !level.is_finite()
            || !distance_blocks.is_finite()
            || distance_blocks > h.bank_raise_blocks
        {
            return;
        }
        let shortfall = level - land_y;
        if shortfall > 0.0 && shortfall <= h.bank_raise_max_blocks {
            target = target.max(level);
        }
    };
    if let Some(hit) = hydrology.channels.nearest {
        if let Some(level) = hit.level_blocks {
            // Расстояние — от кромки воды, у реки это `core/2` от оси.
            let edge = 0.5 * hit.core_l * l;
            consider(level, (hit.distance_l * l - edge).max(0.0));
        }
    }
    if let Some(hit) = hydrology.lakes.water_hit.filter(|hit| hit.inside) {
        if let Some(level) = hit.level_blocks {
            consider(level, hit.distance_l * l);
        }
    }
    if target > land_y {
        height.surface_y = target;
        height.offset = (target - SURFACE_BASE) / OFFSET_SCALE;
    }
    height
}

/// ВО1-2 (6.5, `PLAN-WATER-UPDATE.md`): вырез итоговой поверхности `W`.
///
/// `s0_macro` — `S0` без микробугров (строка таблицы высот `W`,
/// `macro_y`/`sample_micro(…, 0.0, 0.0, …)`); `micro_dy` — вклад микробугров
/// отдельным слагаемым (`S0 = S0_macro + micro`, 6.1). Поверхность —
/// **только ниже** `S0`: `min(S0_macro + micro·gate, V_river…, V_озеро,
/// V_feature)`; озёра — как в R2 (6.4, ВО1–ВО2), подпор/русло по профилю W.
/// `raise_bank` добивается ≤ 2 блока, как в R2.
///
/// Параметры берутся из `params.water` (6.9) и классов хита
/// (`valley_class`, `flow_class`, `gp`): каньон — ступенями
/// `canyon_step_*`, ледниковая U — `1 + x²/8` до 16 блоков и дальше по
/// 4 блока на блок вбок, иначе `s_wall(class)` с максимумом
/// `(Gp − W − 1)/R_wall`. `gate` микробугров — `smoothstep` по `d_floor`.
/// Русло без уровня `level_blocks` (не считался/нет профиля) не вырезается.
#[allow(clippy::too_many_lines)]
pub fn with_water_w(
    s0_macro: HeightSample,
    micro_dy: f64,
    water: &HydrologySample,
    params: &Params,
) -> HeightSample {
    let w = &params.water;
    let h = &params.height;
    let l = params.block.l_blocks;
    let sea = SEA_LEVEL;
    let land_y = s0_macro.surface_y;
    let s0_full = land_y + micro_dy;
    // Собираем звенья-кандидаты: ближайшее плюс до двух из коридора 6.5.
    let mut candidates: Vec<crate::channel::ChannelHit> = Vec::with_capacity(4);
    if let Some(hit) = water.channels.nearest {
        candidates.push(hit);
    }
    for hit in water.channels.extra.iter() {
        candidates.push(*hit);
    }
    // Дубль ближайшего звена в `extra` допустим: min() от него не меняется.
    let (gate_lo, gate_hi) = (w.gate_blocks[0], w.gate_blocks[1]);
    let scientific = matches!(params.mode, crate::mode::Mode::Scientific);
    let depth_min = 2.0_f64;
    let depth_max = if scientific { 8.0 } else { 5.0 };
    let mut best: Option<(f64, f64, f64, f64)> = None; // (v_y, water_y, bed_y, strength)
    let mut d_floor_min = f64::INFINITY;
    for hit in &candidates {
        let Some(level) = hit.level_blocks else {
            continue;
        };
        let w_blocks = (hit.core_l * l).max(1.0);
        let half = 0.5 * w_blocks;
        let bank_edge = half + 2.0;
        let d_abs = hit.distance_l * l;
        let fp = w.floodplain_w[hit.valley_class.table_index()] * w_blocks;
        let dp = d_abs - bank_edge;
        let d_floor = dp - fp;
        if d_floor < d_floor_min {
            d_floor_min = d_floor;
        }
        let s_wall = w.wall_slope[hit.valley_class.table_index()];
        let s_eff = if hit.gp > 0.0 {
            s_wall.max((hit.gp - level - 1.0) / w.wall_reach_blocks)
        } else {
            s_wall
        };
        let candidate = if d_abs <= half {
            // Ядро |d| ≤ w/2: параболическое дно под зеркалом.
            let depth = hit.depth_blocks.clamp(depth_min, depth_max);
            let shape = (1.0 - (2.0 * d_abs / w_blocks).powi(2)).max(0.35);
            level - depth * shape
        } else if d_abs <= bank_edge {
            // Береговая полоса 2 блока: вровень; у равнинной реки при w ≥ 8 —
            // прирусловый вал +1, если `S0` выше.
            let mut y = level;
            let plain = hit.valley_class == crate::water_profile::ValleyClass::Plain;
            if plain && w_blocks >= 8.0 && s0_full > level + 1.0 {
                y += 1.0;
            }
            y
        } else if dp <= fp {
            // Пойма: +1 над зеркалом.
            level + 1.0
        } else {
            let x = d_floor; // d' − fp ≥ 0
            match hit.valley_class {
                crate::water_profile::ValleyClass::Canyon => {
                    // Ступени каньона: +1 + H_step · ceil(x / L_step).
                    level + 1.0 + w.canyon_step_blocks * (x / w.canyon_step_every).ceil()
                }
                crate::water_profile::ValleyClass::GlacialU => {
                    // Парабола до 16 блоков, дальше 4 блока вверх на блок вбок.
                    if x <= 16.0 {
                        level + 1.0 + x * x / 8.0
                    } else {
                        level + 1.0 + 16.0 * 16.0 / 8.0 + 4.0 * (x - 16.0)
                    }
                }
                _ => {
                    // Обычный склон: +1 + s_eff·(d' − fp).
                    level + 1.0 + s_eff * x
                }
            }
        };
        // Кандидат участвует в min() безусловно: сравнение с S0 — ниже.
        let bed = candidate;
        let strength = if d_abs <= half {
            1.0
        } else if dp <= 0.0 {
            0.5
        } else {
            0.0
        };
        let take = match best {
            Some((old, _, _, _)) => candidate < old,
            None => true,
        };
        if take {
            best = Some((candidate, level, bed, strength));
        }
    }
    // Итоговая поверхность: S0 с гаснущими у воды микробуграми, не выше неё.
    let gate = if d_floor_min.is_finite() {
        smoothstep(gate_lo, gate_hi, d_floor_min)
    } else {
        1.0
    };
    let mut surface = land_y + micro_dy * gate;
    let mut water_y = sea;
    let mut bed_y = surface;
    let mut cut = 0.0_f64;
    if let Some((v_y, mirror, bed, strength)) = best {
        if v_y < surface {
            surface = v_y;
            bed_y = bed;
            cut = strength;
            // Вода стоит, где дно ниже зеркала; береговая полоса — вровень,
            // поэтому у самой кромки воды нет.
            if v_y < mirror {
                water_y = mirror;
            }
        }
    }
    let mut height = HeightSample {
        surface_y: surface,
        water_y,
        bed_y,
        water_strength: cut,
        ..s0_macro
    };
    height.offset = (surface - SURFACE_BASE) / OFFSET_SCALE;
    // Озёра — правило R2 (6.4, ВО1–ВО2): плоское зеркало min(S) по контуру −1.
    if let Some(hit) = water.lakes.water_hit.filter(|hit| hit.inside) {
        if let Some(mirror) = hit.level_blocks.filter(|level| *level >= h.min_y) {
            let lake_cut = clamp01(hit.water_strength);
            if lake_cut > 0.0 && surface < mirror {
                let depth = clamp(
                    hit.max_depth_km * h.water_km_to_blocks,
                    1.0,
                    h.lake_depth_max_blocks,
                );
                let bed = (mirror - depth).max(h.min_y);
                let carve = lerp(surface, bed, lake_cut).min(surface);
                if carve < mirror - 0.5 {
                    surface = carve;
                    water_y = mirror;
                    bed_y = bed;
                    cut = lake_cut;
                    height.surface_y = surface;
                    height.water_y = water_y;
                    height.bed_y = bed_y;
                    height.water_strength = cut;
                    height.offset = (surface - SURFACE_BASE) / OFFSET_SCALE;
                }
            }
        }
    }
    // ВО2-1 (§6.7): маска calm — ноль в воде и у воды, снаружи затухает
    // smoothstep-ом по d_floor; для озёра действует тот же профиль (паду у
    // озера можно расширить зону позже, сейчас d_floor от русла покрывает
    // близость к любой воде).
    let (calm_lo, calm_hi) = (w.calm_blocks[0], w.calm_blocks[1]);
    let in_water = cut > 0.0;
    let calm = if in_water {
        0.0
    } else if d_floor_min.is_finite() {
        smoothstep(calm_lo, calm_hi, d_floor_min)
    } else {
        1.0
    };
    height.calm = calm;

    if cut > 0.0 {
        return height;
    }
    // Вода не выбрала колонку: добивка берега как в R2 (≤ `bank_raise_max_blocks`).
    raise_bank(height, water, params, 0.0)
}

pub fn validate(params: &Params) -> Result<(), &'static str> {
    let h = &params.height;
    let increasing = |xs: &[f64]| xs.windows(2).all(|w| w[0].is_finite() && w[1] > w[0]);
    if !increasing(&h.e_knots) || !increasing(&h.pv_knots) || !increasing(&h.ocean_c) {
        return Err("height: узлы таблиц должны строго возрастать");
    }
    if h.land_y.iter().flatten().any(|v| !v.is_finite())
        || h.ocean_y.iter().any(|v| !v.is_finite())
        || h.factor
            .iter()
            .any(|v| !v.is_finite() || !(0.5..=6.5).contains(v))
        || h.jaggedness
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err("height: таблицы высот/factor/jaggedness должны быть конечными");
    }
    if !h.coast_width_c.is_finite()
        || h.coast_width_c <= 0.0
        || !h.shore_width_blocks.is_finite()
        || h.shore_width_blocks < 0.0
        || !h.shore_cliff_e.is_finite()
        || !(-1.0..=1.0).contains(&h.shore_cliff_e)
        || !h.e_gain_blocks_per_km.is_finite()
        || !h.e_gain_base_km.is_finite()
        || h.e_gain_base_km < 0.0
        || h.micro_amp_blocks
            .iter()
            .any(|v| !v.is_finite() || *v < 0.0)
        || !h.micro_lambda_blocks.is_finite()
        || h.micro_lambda_blocks <= 0.0
        || !h.micro_low_y.is_finite()
        || !h.micro_high_y.is_finite()
        || h.micro_high_y <= h.micro_low_y
        || !h.cold_jagged_gain.is_finite()
        || !(0.0..=1.0).contains(&h.cold_jagged_gain)
        || !h.warm_jagged_drop.is_finite()
        || !(0.0..=1.0).contains(&h.warm_jagged_drop)
        || !h.min_y.is_finite()
        || !h.max_y.is_finite()
        || h.min_y >= h.max_y
        || !h.water_km_to_blocks.is_finite()
        || h.water_km_to_blocks <= 0.0
        || !h.lake_depth_max_blocks.is_finite()
        || h.lake_depth_max_blocks < 1.0
        || !h.bank_raise_blocks.is_finite()
        || !(0.0..=256.0).contains(&h.bank_raise_blocks)
        || !h.bank_raise_max_blocks.is_finite()
        || !(0.0..=256.0).contains(&h.bank_raise_max_blocks)
    {
        return Err("height: переход берега/вертикальный масштаб/пределы некорректны");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::Mode;
    use crate::version::GenVersion;

    fn params(mode: Mode) -> Params {
        Params::defaults(GenVersion::V1Dev, mode)
    }

    /// R2 (§4): уровень воды считается от сухой поверхности, вырез только вниз.
    fn params_r2(mode: Mode) -> Params {
        Params::defaults(GenVersion::V1DevR2, mode)
    }

    fn channel_level(
        mirror_y: f64,
        depth_blocks: f64,
        level: Option<f64>,
    ) -> crate::channel::ChannelHit {
        let mut hit = channel_hit(mirror_y, depth_blocks);
        hit.level_blocks = level;
        hit
    }

    fn params_w(mode: Mode) -> Params {
        Params::defaults(GenVersion::V1DevW, mode)
    }

    fn w_hit(
        core_blocks: f64,
        distance_blocks: f64,
        level: f64,
        class: crate::water_profile::ValleyClass,
        gp: f64,
    ) -> crate::channel::ChannelHit {
        let l = Mode::Gameplay.l_blocks();
        let mut hit = channel_level(70.0, 2.0, Some(level));
        hit.core_l = core_blocks / l;
        hit.distance_l = distance_blocks / l;
        hit.valley_class = class;
        hit.gp = gp;
        hit
    }

    fn s0(surface_y: f64) -> HeightSample {
        HeightSample {
            surface_y,
            offset: (surface_y - SURFACE_BASE) / OFFSET_SCALE,
            bed_y: surface_y,
            ..HeightSample::default()
        }
    }

    #[test]
    fn w_core_cuts_parabolic_bed_below_level() {
        let p = params_w(Mode::Gameplay);
        let mut hydro = HydrologySample::default();
        // Ядро w=8: d=0 → дно на depth ниже зеркала; d=2 → на 0,75·depth.
        hydro.channels.nearest = Some(w_hit(
            8.0,
            0.0,
            70.0,
            crate::water_profile::ValleyClass::Plain,
            0.0,
        ));
        let cut = with_water_w(s0(90.0), 0.0, &hydro, &p);
        assert_eq!(cut.surface_y, 70.0 - 2.0, "параболическое дно ядра");
        assert_eq!(cut.water_y, 70.0);
        assert_eq!(cut.water_strength, 1.0);
        hydro.channels.nearest = Some(w_hit(
            8.0,
            2.0,
            70.0,
            crate::water_profile::ValleyClass::Plain,
            0.0,
        ));
        let cut = with_water_w(s0(90.0), 0.0, &hydro, &p);
        assert_eq!(cut.surface_y, 70.0 - 2.0 * 0.75, "(2d/w)² = 0,25");
    }

    #[test]
    fn w_valley_floor_is_cut_to_level_plus_one() {
        let p = params_w(Mode::Gameplay);
        let mut hydro = HydrologySample::default();
        // Равнинная река w=8: fp=3·8=24; до поймы d' ≤ 24 блоков — дно µ +1.
        hydro.channels.nearest = Some(w_hit(
            8.0,
            7.0,
            70.0,
            crate::water_profile::ValleyClass::Plain,
            0.0,
        ));
        let cut = with_water_w(s0(90.0), 7.0, &hydro, &p);
        assert_eq!(cut.surface_y, 71.0, "пойма: +1 над зеркалом");
        assert_eq!(cut.water_strength, 0.0, "пойма — суша");
        assert_eq!(cut.water_y, SEA_LEVEL);
        // Микробугры у воды гаснут: gate=0 (d_floor < 0).
        assert_eq!(cut.offset, (71.0 - SURFACE_BASE) / OFFSET_SCALE);
    }

    #[test]
    fn w_canyon_walls_step_up() {
        let p = params_w(Mode::Gameplay);
        let mut hydro = HydrologySample::default();
        // Каньон: fp = 0,5·w = 4 блока; d' = 6 → x = 2 → 1 + 6·ceil(2/2) = 7.
        hydro.channels.nearest = Some(w_hit(
            8.0,
            4.0 + 2.0 + 6.0,
            70.0,
            crate::water_profile::ValleyClass::Canyon,
            90.0,
        ));
        let cut = with_water_w(s0(200.0), 0.0, &hydro, &p);
        assert_eq!(cut.surface_y, 70.0 + 1.0 + 6.0 * (2.0_f64 / 2.0).ceil());
    }

    #[test]
    fn w_micro_gate_smooths_out_far_from_water() {
        let p = params_w(Mode::Gameplay);
        let hydro_dry = HydrologySample::default(); // канала нет: gate = 1
        let cut = with_water_w(s0(90.0), 7.0, &hydro_dry, &p);
        assert_eq!(cut.surface_y, 97.0);
        let mut hydro = HydrologySample::default();
        hydro.channels.nearest = Some(w_hit(
            8.0,
            7.0,
            70.0,
            crate::water_profile::ValleyClass::Plain,
            0.0,
        ));
        // Рядом с руслом (d_floor < 0) микробугры выключены, но поверхность
        // выравнивается полкой поймы (71).
        let cut = with_water_w(s0(90.0), 7.0, &hydro, &p);
        assert_eq!(cut.surface_y, 71.0);
    }

    fn lake_level(level_km: f64, max_depth_km: f64, level: Option<f64>) -> crate::lake::LakeHit {
        let mut hit = lake_hit(level_km, max_depth_km);
        hit.level_blocks = level;
        hit
    }

    #[test]
    fn r2_river_takes_level_from_banks_and_cuts_only_down() {
        let p = params_r2(Mode::Gameplay);
        let mut hydro = HydrologySample::default();
        hydro.channels.core = 1.0;
        hydro.channels.bank = 1.0;
        hydro.channels.nearest = Some(channel_level(120.0, 2.0, Some(80.0)));
        // Суша 79 при зеркале 80 и глубине 2: вырез опускает дно до 78,
        // но не выше суши (79) и не выше зеркала минус блок.
        let cut = with_water(base_at(79.0), &hydro, &p);
        assert_eq!(
            cut.water_y, 80.0,
            "уровень взят от берегов, а не из километров"
        );
        assert!(
            cut.surface_y <= 79.0,
            "дно {:?} не ниже зеркала",
            cut.surface_y
        );
        assert!(cut.surface_y <= 79.0, "вырез поднял рельеф");
        // Суша 90 выше зеркала 80: колонка остаётся сушей (§4, п. 3).
        let high = with_water(base_at(90.0), &hydro, &p);
        assert_eq!(high.water_strength, 0.0, "вода встала над сушей");
        assert_eq!(high.surface_y, 90.0);
    }

    /// §4, п. 3: река не выкапывает каньон под зеркало, которого на
    /// поверхности нет. Суша 90, зеркало 62 — разница 28 блоков, это не
    /// русло: колонка остаётся сушей, уровень считается ошибкой, а не вырезом.
    #[test]
    fn r2_river_does_not_dig_a_canyon_to_the_mirror() {
        let p = params_r2(Mode::Gameplay);
        let mut hydro = HydrologySample::default();
        hydro.channels.core = 1.0;
        hydro.channels.bank = 1.0;
        hydro.channels.nearest = Some(channel_level(120.0, 2.0, Some(62.0)));
        let column = with_water(base_at(90.0), &hydro, &p);
        assert_eq!(column.water_strength, 0.0, "вырыт каньон под зеркало");
        assert_eq!(column.surface_y, 90.0);
        // Суша 70 при зеркале 71 и глубине 2: дно 69, вода стоит.
        hydro.channels.nearest = Some(channel_level(120.0, 2.0, Some(71.0)));
        let shallow = with_water(base_at(70.0), &hydro, &p);
        assert_eq!(shallow.water_y, 71.0);
        assert!(shallow.surface_y <= 70.0, "дно {:?}", shallow.surface_y);
    }

    #[test]
    fn r2_without_surface_level_gives_no_water_above_sea() {
        let p = params_r2(Mode::Gameplay);
        let mut hydro = HydrologySample::default();
        hydro.channels.core = 1.0;
        hydro.channels.bank = 1.0;
        hydro.channels.nearest = Some(channel_level(120.0, 2.0, None));
        let out = with_water(base_at(100.0), &hydro, &p);
        assert_eq!(out.water_strength, 0.0);
        assert_eq!(out.surface_y, 100.0);
        assert_eq!(out.water_y, SEA_LEVEL);
    }

    /// Игровой тест 2026-11-19: русло на уровне моря (берег 63,5, уровня
    /// R2 нет) режется как в v1-dev, иначе в игре вместо реки лужи.
    #[test]
    fn r2_sea_level_river_is_cut_below_vanilla_water() {
        let p = params_r2(Mode::Gameplay);
        let mut hydro = HydrologySample::default();
        hydro.channels.core = 1.0;
        hydro.channels.bank = 1.0;
        hydro.channels.nearest = Some(channel_level(63.0, 1.93, None));
        for land in [63.5, 63.9] {
            let out = with_water(base_at(land), &hydro, &p);
            assert!(out.water_strength > 0.0, "суша {land}: русло не вырезано");
            assert!(
                out.surface_y < SEA_LEVEL - 1.0,
                "суша {land}: дно {} выше воды игры (y 62)",
                out.surface_y
            );
            assert!(
                out.water_y <= SEA_LEVEL,
                "уровень {} выше моря",
                out.water_y
            );
            assert!(out.surface_y >= land - 3.0, "каньон: {}", out.surface_y);
        }
    }

    /// Игровой тест 2026-11-19, сид 42 (-6497, -2222): уровень сегмента 74
    /// над сушей 60 давал пласт воды над пустотой. Такая колонка — водопад:
    /// воды R2 нет, уровень берётся у моря.
    #[test]
    fn r2_level_far_above_land_is_a_fall_not_a_river() {
        let p = params_r2(Mode::Gameplay);
        let mut hydro = HydrologySample::default();
        hydro.channels.core = 1.0;
        hydro.channels.bank = 1.0;
        hydro.channels.nearest = Some(channel_level(120.0, 2.0, Some(74.0)));
        let out = with_water(base_at(60.0), &hydro, &p);
        assert!(
            out.water_y <= SEA_LEVEL,
            "вода {} над сушей 60",
            out.water_y
        );
        assert!(out.surface_y <= 60.0);
        // Уровень на 2 блока выше суши — ещё русло.
        let near = with_water(base_at(72.0), &hydro, &p);
        assert_eq!(near.water_y, 74.0);
    }

    #[test]
    fn r2_lake_keeps_columns_above_the_mirror_as_land() {
        let p = params_r2(Mode::Gameplay);
        let mut hydro = HydrologySample::default();
        hydro.lakes.water_hit = Some(lake_level(0.5, 0.05, Some(90.0)));
        // Остров внутри контура: суша выше зеркала — остаётся сушей.
        let island = with_water(base_at(95.0), &hydro, &p);
        assert_eq!(island.water_strength, 0.0);
        assert_eq!(island.surface_y, 95.0);
        // Котловина: дно ниже зеркала и ниже исходной поверхности.
        let basin = with_water(base_at(88.0), &hydro, &p);
        assert_eq!(basin.water_y, 90.0);
        assert!(basin.surface_y <= 89.0);
        assert!(basin.surface_y <= 88.0);
    }

    #[test]
    fn r2_bank_fill_is_only_a_two_block_top_up() {
        let p = params_r2(Mode::Gameplay);
        let mut hydro = HydrologySample::default();
        hydro.channels.nearest = Some(channel_level(0.0, 2.0, Some(80.0)));
        // Нехватка 1 блок — добивка; 3 блока — уже ошибка уровня, не берега.
        let near = with_water(base_at(79.0), &hydro, &p);
        assert_eq!(near.surface_y, 80.0, "нехватку в блок берег не добрал");
        let far = with_water(base_at(77.0), &hydro, &p);
        assert_eq!(far.surface_y, 77.0, "берег поднят больше чем на 2 блока");
    }

    #[test]
    fn ocean_is_below_sea_and_land_is_above() {
        let p = params(Mode::Gameplay);
        let sea = sample(-0.6, 0.3, 0.0, -2.0, 0.0, 0.0, &p);
        let land = sample(0.4, 0.3, 0.0, 0.5, 0.0, 0.0, &p);
        assert!(sea.surface_y < SEA_LEVEL, "океан {:?}", sea);
        assert!(land.surface_y > SEA_LEVEL, "суша {:?}", land);
    }

    #[test]
    fn negative_pv_cuts_a_plateau_valley() {
        let p = params(Mode::Gameplay);
        let valley = sample(0.4, -0.3, -1.0, 0.5, 0.0, 0.0, &p);
        let peak = sample(0.4, -0.3, 1.0, 0.5, 0.0, 0.0, &p);
        assert!(valley.surface_y < peak.surface_y - 20.0);
        // Д-057: дно долины плато стоит у уровня русла (вариант B), а не
        // уходит каньоном под море.
        assert!(
            (SEA_LEVEL..70.0).contains(&valley.surface_y),
            "долина плато {:?}",
            valley
        );
    }

    #[test]
    fn scientific_mode_adds_real_macro_elevation() {
        let p = params(Mode::Scientific);
        let low = sample(0.4, -0.5, 0.3, 0.0, 0.0, 0.0, &p);
        let high = sample(0.4, -0.5, 0.3, 2.0, 0.0, 0.0, &p);
        assert!(high.surface_y > low.surface_y + 20.0);
        let game = params(Mode::Gameplay);
        let a = sample(0.4, -0.5, 0.3, 0.0, 0.0, 0.0, &game);
        let b = sample(0.4, -0.5, 0.3, 2.0, 0.0, 0.0, &game);
        assert_eq!(a.surface_y.to_bits(), b.surface_y.to_bits());
    }

    #[test]
    fn cold_mountains_are_jaggeder_than_warm_and_fields_are_bounded() {
        let p = params(Mode::Gameplay);
        let cold = sample(0.4, -0.9, 0.7, 1.0, 1.0, 0.0, &p);
        let warm = sample(0.4, -0.9, 0.7, 1.0, 0.0, 1.0, &p);
        assert!(cold.jaggedness > warm.jaggedness);
        for k in 0..200 {
            let t = k as f64 / 199.0;
            let s = sample(
                -1.0 + 2.0 * t,
                -1.0 + 2.0 * t,
                -1.0 + 2.0 * t,
                -4.0 + 8.0 * t,
                t,
                1.0 - t,
                &p,
            );
            assert!(s.surface_y.is_finite());
            assert!((p.height.min_y..=p.height.max_y).contains(&s.surface_y));
            assert!((0.5..=6.5).contains(&s.factor));
            assert!((0.0..=1.0).contains(&s.jaggedness));
            assert_eq!(
                s.offset.to_bits(),
                ((s.surface_y - SURFACE_BASE) / OFFSET_SCALE).to_bits()
            );
        }
    }

    #[test]
    fn offset_matches_game_depth_formula() {
        // К2: в игре глубина на высоте y равна (1 − y/128) + offset.
        // На y=80 градиент равен 0,375, поэтому глубина = 0,375 + offset,
        // а поверхность (глубина 0) лежит ровно на surface_y.
        let p = params(Mode::Gameplay);
        for k in 0..200 {
            let t = k as f64 / 199.0;
            let s = sample(
                -1.0 + 2.0 * t,
                -1.0 + 2.0 * t,
                -1.0 + 2.0 * t,
                -4.0 + 8.0 * t,
                t,
                1.0 - t,
                &p,
            );
            let depth_at_80 = (1.0 - 80.0 / OFFSET_SCALE) + s.offset;
            assert!((depth_at_80 - (0.375 + s.offset)).abs() < 1e-12);
            let surface_from_offset = OFFSET_SCALE * (1.0 + s.offset);
            assert!(
                (surface_from_offset - s.surface_y).abs() < 1e-9,
                "поверхность {surface_from_offset} != {s:?}"
            );
        }
        // Явные точки из разбора К2: дно океана, берег, равнина, горы.
        for y in [35.0, 63.0, 70.0, 150.0] {
            let offset = y / OFFSET_SCALE - 1.0;
            assert!((OFFSET_SCALE * (1.0 + offset) - y).abs() < 1e-12);
        }
    }

    #[test]
    fn height_keys_are_v1_dev_only_and_validate() {
        let old = Params::defaults(GenVersion::V0, Mode::Gameplay);
        assert!(!old.canonical_text().contains("height."));
        let dev = params(Mode::Gameplay);
        assert!(dev.canonical_text().contains("height.land_y="));
        validate(&dev).unwrap();
        let mut bad = dev.clone();
        bad.height.coast_width_c = 0.0;
        assert!(validate(&bad).is_err());
        let mut bad = dev.clone();
        bad.height.shore_width_blocks = -1.0;
        assert!(validate(&bad).is_err());
    }

    fn base_at(y: f64) -> HeightSample {
        HeightSample {
            surface_y: y,
            offset: (y - SURFACE_BASE) / OFFSET_SCALE,
            bed_y: y,
            ..HeightSample::default()
        }
    }

    fn channel_hit(mirror_y: f64, depth_blocks: f64) -> crate::channel::ChannelHit {
        crate::channel::ChannelHit {
            plate_id: 1,
            segment: 0,
            reach: 0,
            distance_l: 0.0,
            discharge_mm_l2: 100.0,
            core_l: 0.02,
            valley_l: 0.1,
            depth_blocks,
            core_strength: 1.0,
            valley_strength: 1.0,
            delta_strength: 0.0,
            end: crate::channel::ReachEnd::Confluence,
            mouth_distance_l: 1.0,
            mirror_y,
            level_blocks: None,
            along: 0.0,
            side: 1.0,
            valley_class: crate::water_profile::ValleyClass::Plain,
            flow_class: crate::water_profile::FlowClass::Calm,
            gp: 0.0,
        }
    }

    #[test]
    fn river_cut_keeps_water_below_the_local_bank() {
        let p = params(Mode::Gameplay);
        let mut hydro = HydrologySample::default();
        hydro.channels.core = 1.0;
        hydro.channels.bank = 1.0;
        hydro.channels.nearest = Some(channel_hit(80.0, 2.0));
        let cut = with_water(base_at(100.0), &hydro, &p);
        assert_eq!(cut.water_y, 80.0);
        assert_eq!(cut.bed_y, 78.0);
        assert_eq!(cut.surface_y, 78.0);
        assert_eq!(cut.water_strength, 1.0);
        // Берег ниже зеркала: вода не должна висеть над берегом.
        let low = with_water(base_at(70.0), &hydro, &p);
        assert_eq!(low.water_y, 69.0);
        assert!(low.surface_y <= low.water_y - 1.0);
    }

    fn lake_hit(level_km: f64, max_depth_km: f64) -> crate::lake::LakeHit {
        crate::lake::LakeHit {
            plate_id: 1,
            lake: 0,
            depression: 0,
            kind: crate::lake::LakeKind::Lake,
            distance_l: 0.0,
            inside: true,
            level_km: Some(level_km),
            max_depth_km,
            frozen: false,
            rift: false,
            high: level_km > 0.0,
            water_strength: 1.0,
            shore_strength: 1.0,
            level_blocks: None,
        }
    }

    #[test]
    fn lake_mirror_is_flat_at_spill_and_bed_is_below_it() {
        let p = params(Mode::Gameplay);
        let mut hydro = HydrologySample::default();
        hydro.lakes.water_hit = Some(lake_hit(0.1, 0.05));
        let cut = with_water(base_at(80.0), &hydro, &p);
        assert_eq!(cut.water_y, 69.0); // 63 + 0.1*64 = 69.4 -> 69
        assert!((cut.bed_y - 65.8).abs() < 1e-9);
        assert_eq!(cut.surface_y, cut.bed_y);
        assert_eq!(cut.water_strength, 1.0);
    }

    #[test]
    fn deferred_lake_leaves_terrain_untouched() {
        // К3: уровень 0,5 км выше r1_max_level_km (0,3) — вода ждёт R2,
        // рельеф не вырезать и не поднимать (Д-102).
        let p = params(Mode::Gameplay);
        let mut hydro = HydrologySample::default();
        hydro.lakes.water_hit = Some(lake_hit(0.5, 0.05));
        let out = with_water(base_at(80.0), &hydro, &p);
        assert_eq!(out.surface_y, 80.0);
        assert_eq!(out.water_strength, 0.0);
        assert_eq!(out.bed_y, 80.0);
        assert_eq!(out.water_y, SEA_LEVEL);
    }

    #[test]
    fn r1_lake_water_never_stands_above_the_land() {
        // К3: низменная котловина (суша 64) с зеркалом по уровню 0,2 км
        // (63 + 12,8 ≈ 76). Без зажима берегом дно встало бы столом на
        // 72,8 — выше суши, стена до ~26 блоков (Д-102).
        let p = params(Mode::Gameplay);
        let mut hydro = HydrologySample::default();
        hydro.lakes.water_hit = Some(lake_hit(0.2, 0.05));
        let cut = with_water(base_at(64.0), &hydro, &p);
        assert_eq!(cut.water_y, 63.0); // 76 зажат берегом: min(76, 63)
        assert!(cut.bed_y <= 63.0, "дно {:?} выше суши", cut.bed_y);
        assert!(cut.bed_y < cut.water_y);
        assert_eq!(cut.surface_y, cut.bed_y);
        // На обычном берегу (суша выше зеркала) зажим не мешает.
        let high = with_water(base_at(80.0), &hydro, &p);
        assert_eq!(high.water_y, 76.0); // 63 + 0.2*64 = 75.8 -> 76
    }

    /// К4: у берега, где окно C сжато крутым шельфом, суша всё равно
    /// спускается к воде на `shore_width_blocks` от изолинии.
    #[test]
    fn shore_ramp_descends_to_the_water_within_width() {
        let p = params(Mode::Gameplay);
        let h = &p.height;
        // Равнина у берега: окно C уже пройдено (coast_c = 1), до берега 2 блока.
        let c = p.climate.ocean_c_threshold + 2.0 * h.coast_width_c;
        let land = sample_micro(c, 0.3, 0.0, 0.1, 0.0, 0.0, 0.0, 0.0, f64::INFINITY, &p);
        let shore = sample_micro(c, 0.3, 0.0, 0.1, 0.0, 0.0, 0.0, 0.0, 2.0, &p);
        assert!(
            land.surface_y > shore.surface_y + 5.0,
            "рампа не срезала берег: суша {:?}, берег {:?}",
            land.surface_y,
            shore.surface_y
        );
        assert!(
            (SEA_LEVEL - 1.0..SEA_LEVEL + 3.0).contains(&shore.surface_y),
            "берег {:?} не у воды",
            shore.surface_y
        );
        // Дальше ширины спуска рампа не действует.
        let far = sample_micro(
            c,
            0.3,
            0.0,
            0.1,
            0.0,
            0.0,
            0.0,
            0.0,
            4.0 * h.shore_width_blocks,
            &p,
        );
        assert_eq!(far.surface_y, land.surface_y);
    }

    /// К4: горный берег (низкая эрозия) обрыв сохраняет — рампа не действует.
    #[test]
    fn mountain_shore_keeps_the_cliff() {
        let p = params(Mode::Gameplay);
        let c = p.climate.ocean_c_threshold + 2.0 * p.height.coast_width_c;
        let far = sample_micro(c, -0.6, 0.0, 0.1, 0.0, 0.0, 0.0, 0.0, f64::INFINITY, &p);
        let shore = sample_micro(c, -0.6, 0.0, 0.1, 0.0, 0.0, 0.0, 0.0, 2.0, &p);
        assert_eq!(far.surface_y, shore.surface_y);
        assert!(shore.surface_y > SEA_LEVEL + 10.0);
    }
}
