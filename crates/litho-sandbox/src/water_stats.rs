//! ВО0-3 (PLAN-WATER-UPDATE §10.1): метрика воды `water-stats`.
//!
//! Считается по сетке колонок окна: реки (доля воды выше моря, врез и разброс
//! перепада), озёра как **отдельные водоёмы** (связные группы колонок воды), а
//! не площадь узлов, и стены у воды. Числа — база для сравнения этапов ВО1–ВО9:
//! на `v1-dev-r2` они снимаются сейчас, на `v1-dev-w` — после каждого шага.
//!
//! Сетка честно грубее блоков: `--step` и `--window` задаёт вызывающий. Окно
//! по умолчанию меньше планового ±8000 / ±24000: полный объём на `step = 8`
//! — это миллионы колонок, и в CI он идёт отдельной задачей. Плановые числа
//! задаются флагами `--window 8000 --step 8` (геймплей) и
//! `--window 24000 --step 16` (научный).

use std::collections::VecDeque;

use litho_core::Generator;
use litho_core::SurfaceKind;
use litho_core::generator::WaterColumn;
use litho_core::height::SEA_LEVEL;
use litho_core::lake::LakeKind;
use litho_core::morphoclimate::ZONE_NAMES;
use litho_core::params::MORPHO_ZONES;

/// Уровень моря в блоках: вода выше него — предмет водного обновления.
const SEA: i32 = SEA_LEVEL as i32;

/// Итог `water-stats` по окну.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub land: u64,
    /// Колонок сетки в русле.
    pub river_columns: u64,
    /// Из них с водой выше моря (`level ≥ 64`).
    pub river_above_sea: u64,
    /// Врез `сухая поверхность − level` на реках: p50 / p90 / p99, блоки.
    pub gorge_p50: f64,
    pub gorge_p90: f64,
    pub gorge_p99: f64,
    /// Отдельные водоёмы (связная группа ≥ `min_lake_blocks²` площади).
    pub lakes: u64,
    /// Озёр на 100 L² суши (10.1).
    pub lakes_per_100l2: f64,
    /// Площадь воды озёр от площади суши, %.
    pub lake_area_pct: f64,
    /// Доля площади озёр, где сухая поверхность ниже уровня («в котловине»), %.
    pub lake_fit_pct: f64,
    /// Озёр с уровнем ≥ 80.
    pub lakes_ge_80: u64,
    /// Доля колонок суши в 2 блоках от воды со стеной `surface − level ≥ 4`.
    pub walls_pct: f64,
    /// Доля колонок коридора воды (ядро, берег, пойма по `info`) с
    /// `|верхний твёрдый блок − поверхность ядра| ≤ 2` — гейт calm_fit 8.3.
    pub calm_fit_pct: f64,
    /// Доля колонок с водой, над которыми стоит твёрдый блок (накрытая вода)
    /// — гейт 8.3, должен быть ≤ 0.5 %.
    pub covered_water_pct: f64,
    /// Озёра по доминирующей морфозоне: штук и % площади суши зоны.
    pub zone_lakes: [u64; MORPHO_ZONES],
    /// Суши (колонок сетки) по доминирующей морфозоне — для плотности озёр по зоне.
    pub zone_land: [u64; MORPHO_ZONES],
    /// Площадь воды озёр по доминирующей морфозоне, блоков².
    pub zone_lake_area: [f64; MORPHO_ZONES],
    /// Затопленная суша: доля колонок суши (`S0 ≥ 64`), чья сухая поверхность
    /// ниже уровня моря или любого ближайшего водоёма (потенциально подтопляемая), %.
    pub flooded_land_pct: f64,
    /// ВО4-4: озёра по видам (LakeKind): штук. Индексы совпадают с порядком
    /// вариантов `LakeKind` (0=Lake, 1=InlandSea, 2=Swamp, 3=DryBasin,
    /// 4=ClosedSurplus, 5=SeaConnected, 6=Rift, 7=Highland).
    pub lakes_by_kind: [u64; 8],
}

impl Stats {
    pub fn zone_names(&self) -> [&'static str; MORPHO_ZONES] {
        ZONE_NAMES
    }
}

/// Ячейка сетки: что в колонке и её высоты.
#[derive(Clone, Copy, Default)]
struct Cell {
    /// Вода озера/внутреннего моря/болота выше моря.
    lake: bool,
    river: bool,
    /// Вода выше моря (любая).
    water: bool,
    level: i32,
    /// Верхний твёрдый блок колонки (поверхность после выреза).
    surface: i32,
    /// Дно под зеркалом.
    bed: i32,
    /// Сухая поверхность колонки (`S`), блоки.
    dry: f32,
    zone: u8,
    /// ВО4-4: вид озера, если колонка — озеро выше моря.
    lake_kind: Option<LakeKind>,
}

const NO_ZONE: u8 = u8::MAX;

fn is_land(dry: f64) -> bool {
    dry >= (SEA as f64 + 1.0)
}

/// Одна сетка окна: колонки, затем связные группы воды.
pub struct Window {
    width: usize,
    height: usize,
    step: i64,
    cells: Vec<Cell>,
}

impl Window {
    pub fn new(
        generator: &Generator,
        center: (f64, f64),
        window: i64,
        step: i64,
    ) -> Result<Self, String> {
        let step = step.max(1);
        let half = window.max(step) / 2;
        let x0 = center.0 as i64 - half;
        let z0 = center.1 as i64 - half;
        let width = (2 * half / step + 1) as usize;
        let height = width;
        let mut cells = Vec::with_capacity(width * height);
        for j in 0..height {
            let z = z0 + j as i64 * step;
            for i in 0..width {
                let x = x0 + i as i64 * step;
                cells.push(sample_cell(generator, x, z));
            }
        }
        Ok(Self {
            width,
            height,
            step,
            cells,
        })
    }

    fn at(&self, i: usize, j: usize) -> Cell {
        self.cells[j * self.width + i]
    }

    /// Замыкает окно в метрики §10.1.
    pub fn stats(&self, mode_scientific: bool) -> Stats {
        let mut out = Stats::default();
        let step_area = (self.step * self.step) as f64;
        let mut lake_area = 0.0f64;
        let mut lake_fit_area = 0.0f64;
        let mut gorge: Vec<f64> = Vec::new();
        // Минимальная площадь водоёма: 64 / 256 блоков² (§10.1).
        let min_area = if mode_scientific { 256.0 } else { 64.0 };

        for j in 0..self.height {
            for i in 0..self.width {
                let cell = self.at(i, j);
                if is_land(cell.dry as f64) {
                    out.land += 1;
                    if (cell.zone as usize) < MORPHO_ZONES {
                        out.zone_land[cell.zone as usize] += 1;
                    }
                }
                if cell.river {
                    out.river_columns += 1;
                    if cell.water && cell.level > SEA {
                        out.river_above_sea += 1;
                    }
                    if cell.water {
                        gorge.push(cell.dry as f64 - cell.level as f64);
                    }
                }
            }
        }

        // Отдельные водоёмы: связные по четырём соседям группы колонок воды.
        let mut visited = vec![false; self.cells.len()];
        let mut queue = VecDeque::new();
        for j in 0..self.height {
            for i in 0..self.width {
                let index = j * self.width + i;
                if visited[index] || !self.cells[index].lake {
                    continue;
                }
                let mut count = 0u64;
                let mut fit_cells = 0u64;
                let mut level_max = i32::MIN;
                let mut zone: Option<usize> = None;
                let mut lake_kind: Option<LakeKind> = None;
                visited[index] = true;
                queue.push_back((i, j));
                while let Some((ci, cj)) = queue.pop_front() {
                    let cell = self.at(ci, cj);
                    count += 1;
                    if (cell.dry as f64) < cell.level as f64 {
                        fit_cells += 1;
                    }
                    level_max = level_max.max(cell.level);
                    if zone.is_none() && (cell.zone as usize) < MORPHO_ZONES {
                        zone = Some(cell.zone as usize);
                    }
                    if lake_kind.is_none() {
                        lake_kind = cell.lake_kind;
                    }
                    let neighbors = [
                        (ci.wrapping_sub(1), cj),
                        (ci + 1, cj),
                        (ci, cj.wrapping_sub(1)),
                        (ci, cj + 1),
                    ];
                    for (ni, nj) in neighbors {
                        if ni >= self.width || nj >= self.height {
                            continue;
                        }
                        let n = nj * self.width + ni;
                        if visited[n] || !self.cells[n].lake {
                            continue;
                        }
                        visited[n] = true;
                        queue.push_back((ni, nj));
                    }
                }
                let area = count as f64 * step_area;
                if area < min_area {
                    continue;
                }
                out.lakes += 1;
                lake_area += area;
                lake_fit_area += fit_cells as f64 * step_area;
                if level_max >= 80 {
                    out.lakes_ge_80 += 1;
                }
                if let Some(zone) = zone {
                    out.zone_lakes[zone] += 1;
                    out.zone_lake_area[zone] += area;
                }
                if let Some(kind) = lake_kind {
                    out.lakes_by_kind[kind.to_index()] += 1;
                }
            }
        }

        // Стены: суша в двух блоках от воды, поверхность выше зеркала на 4+.
        let mut shore_land = 0u64;
        let mut wall_land = 0u64;
        // Затопленная суша: колонки суши, чья сухая поверхность ниже уровня
        // любой соседней воды.
        let mut flooded_land = 0u64;
        for j in 0..self.height {
            for i in 0..self.width {
                let cell = self.at(i, j);
                if cell.water || !is_land(cell.dry as f64) {
                    continue;
                }
                let mut near_water = false;
                let mut wall = false;
                let mut flooded = false;
                for (ni, nj) in [
                    (i.wrapping_sub(1), j),
                    (i + 1, j),
                    (i, j.wrapping_sub(1)),
                    (i, j + 1),
                ] {
                    if ni >= self.width || nj >= self.height {
                        continue;
                    }
                    let n = self.at(ni, nj);
                    if !n.water {
                        continue;
                    }
                    near_water = true;
                    if cell.dry as f64 - n.level as f64 >= 4.0 {
                        wall = true;
                    }
                    if cell.dry as f64 < n.level as f64 {
                        flooded = true;
                    }
                }
                if near_water {
                    shore_land += 1;
                    if wall {
                        wall_land += 1;
                    }
                    if flooded {
                        flooded_land += 1;
                    }
                }
            }
        }

        let land_area = out.land as f64 * step_area;
        let land_l2 = land_area / (128.0 * 128.0);
        out.lakes_per_100l2 = if land_l2 > 0.0 {
            100.0 * out.lakes as f64 / land_l2
        } else {
            0.0
        };
        out.lake_area_pct = if land_area > 0.0 {
            100.0 * lake_area / land_area
        } else {
            0.0
        };
        out.lake_fit_pct = if lake_area > 0.0 {
            100.0 * lake_fit_area / lake_area
        } else {
            0.0
        };
        out.walls_pct = if shore_land > 0 {
            100.0 * wall_land as f64 / shore_land as f64
        } else {
            0.0
        };
        out.flooded_land_pct = if shore_land > 0 {
            100.0 * flooded_land as f64 / shore_land as f64
        } else {
            0.0
        };
        // Гейты 8.3 (ВО2-4): calm_fit и накрытая вода считаются по колонкам
        // коридора воды (воды выше моря, любого вида).
        let mut water_cols = 0u64;
        let mut calm_fit = 0u64;
        let mut covered_water = 0u64;
        for cell in &self.cells {
            if !cell.water {
                continue;
            }
            water_cols += 1;
            // calm_fit: поверхность близка к дну (вырез неглубокий, ≤ 2 блока).
            if (cell.surface - cell.bed).abs() <= 2 {
                calm_fit += 1;
            }
            // Накрытая вода: над зеркалом стоит твёрдый блок.
            if cell.surface > cell.level - 1 {
                covered_water += 1;
            }
        }
        out.calm_fit_pct = if water_cols > 0 {
            100.0 * calm_fit as f64 / water_cols as f64
        } else {
            100.0
        };
        out.covered_water_pct = if water_cols > 0 {
            100.0 * covered_water as f64 / water_cols as f64
        } else {
            0.0
        };
        let mut sorted = gorge;
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        out.gorge_p50 = percentile(&sorted, 0.50);
        out.gorge_p90 = percentile(&sorted, 0.90);
        out.gorge_p99 = percentile(&sorted, 0.99);
        out
    }
}

/// Значение квантиля по уже отсортированному ряду.
fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() as f64 - 1.0) * q).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn sample_cell(generator: &Generator, x: i64, z: i64) -> Cell {
    let (fx, fz) = (x as f64, z as f64);
    let WaterColumn {
        level,
        kind,
        surface,
        bed,
        lake_kind,
        ..
    } = generator.water_column(fx, fz);
    let water = level != WaterColumn::NONE && level > SEA;
    let dry = generator
        .dry_surface_at(fx, fz)
        .ok()
        .flatten()
        .unwrap_or(SEA as f64);
    let zone = generator
        .morpho_weights(fx, fz)
        .map(|weights| {
            let mut best = 0usize;
            for (index, value) in weights.values.iter().enumerate() {
                if *value > weights.values[best] {
                    best = index;
                }
            }
            best as u8
        })
        .unwrap_or(NO_ZONE);
    Cell {
        lake: water
            && matches!(
                kind,
                SurfaceKind::Lake | SurfaceKind::InlandSea | SurfaceKind::Swamp
            ),
        river: kind == SurfaceKind::River,
        water,
        level,
        surface,
        bed,
        dry: dry as f32,
        zone,
        lake_kind,
    }
}
