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
    /// Озёра по доминирующей морфозоне: штук и % площади суши зоны.
    pub zone_lakes: [u64; MORPHO_ZONES],
    pub zone_density: [f64; MORPHO_ZONES],
    pub zone_area_pct: [f64; MORPHO_ZONES],
    pub zone_land: [u64; MORPHO_ZONES],
    pub flooded_land_pct: f64,
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
    /// Сухая поверхность колонки (`S`), блоки.
    dry: f32,
    zone: u8,
    // W: exact 2-block bank test, independent of the window sampling stride.
    wall: Option<bool>,
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
    l_blocks: f64,
    w: bool,
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
            l_blocks: generator.params().block.l_blocks,
            w: generator.params().version.is_water_update(),
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
        let mut zone_water = [0_u64; MORPHO_ZONES];
        let mut flooded = 0;
        let mut gorge: Vec<f64> = Vec::new();
        // Минимальная площадь водоёма: 64 / 256 блоков² (§10.1).
        let min_area = if mode_scientific { 256.0 } else { 64.0 };

        for j in 0..self.height {
            for i in 0..self.width {
                let cell = self.at(i, j);
                if is_land(cell.dry as f64) {
                    out.land += 1;
                    if cell.water {
                        flooded += 1;
                    }
                    if let Some(land) = out.zone_land.get_mut(cell.zone as usize) {
                        *land += 1;
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
                let mut body = Vec::new();
                let (mut sum_i, mut sum_j) = (0.0, 0.0);
                visited[index] = true;
                queue.push_back((i, j));
                while let Some((ci, cj)) = queue.pop_front() {
                    let cell = self.at(ci, cj);
                    count += 1;
                    if (cell.dry as f64) < cell.level as f64 {
                        fit_cells += 1;
                    }
                    level_max = level_max.max(cell.level);
                    body.push((ci, cj));
                    sum_i += ci as f64;
                    sum_j += cj as f64;
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
                // Attribute one body to the sampled cell closest to its centroid
                // (the centroid itself can be dry in a crescent-shaped lake).
                let center = (sum_i / count as f64, sum_j / count as f64);
                let mut nearest = (f64::INFINITY, NO_ZONE);
                for (i, j) in body {
                    let cell = self.at(i, j);
                    let d2 = (i as f64 - center.0).powi(2) + (j as f64 - center.1).powi(2);
                    if d2 < nearest.0 {
                        nearest = (d2, cell.zone);
                    }
                    if let Some(area) = zone_water.get_mut(cell.zone as usize) {
                        *area += 1;
                    }
                }
                if let Some(lakes) = out.zone_lakes.get_mut(nearest.1 as usize) {
                    *lakes += 1;
                }
            }
        }

        // Стены: суша в двух блоках от воды, поверхность выше зеркала на 4+.
        let mut shore_land = 0u64;
        let mut wall_land = 0u64;
        for j in 0..self.height {
            for i in 0..self.width {
                let cell = self.at(i, j);
                if cell.water || !is_land(cell.dry as f64) {
                    continue;
                }
                if self.w {
                    if let Some(wall) = cell.wall {
                        shore_land += 1;
                        wall_land += u64::from(wall);
                    }
                    continue;
                }
                let mut near_water = false;
                let mut wall = false;
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
                }
                if near_water {
                    shore_land += 1;
                    if wall {
                        wall_land += 1;
                    }
                }
            }
        }

        let land_area = out.land as f64 * step_area;
        let l2 = self.l_blocks.powi(2);
        let land_l2 = land_area / l2;
        if out.land > 0 {
            out.flooded_land_pct = 100.0 * flooded as f64 / out.land as f64;
        }
        for (z, &area) in zone_water.iter().enumerate() {
            let land = out.zone_land[z] as f64;
            if land > 0.0 {
                out.zone_density[z] = 100.0 * out.zone_lakes[z] as f64 * l2 / (land * step_area);
                out.zone_area_pct[z] = 100.0 * area as f64 / land;
            }
        }
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
    let column = generator.water_column(fx, fz);
    let WaterColumn { level, kind, .. } = column;
    let water = level != WaterColumn::NONE && level > SEA;
    let dry = if generator.params().version.is_water_update() {
        generator.s0_at(fx, fz).map(|s| s.macro_y)
    } else {
        generator.dry_surface_at(fx, fz).ok().flatten()
    }
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
    let mut wall = None;
    if generator.params().version.is_water_update()
        && level == WaterColumn::NONE
        && column.bed != WaterColumn::NONE
        && is_land(dry)
    {
        // Dry columns in the calm corridor carry their cut surface in `bed`.
        let mut nearby = None;
        let mut exempt = false;
        for dz in -2..=2 {
            for dx in -2..=2 {
                let d2 = dx * dx + dz * dz;
                if !(1..=4).contains(&d2) {
                    continue;
                }
                let n = generator.water_column(fx + f64::from(dx), fz + f64::from(dz));
                if n.level >= SEA {
                    nearby = Some(n.level.max(nearby.unwrap_or(SEA)));
                    exempt |= n.wall_exempt;
                }
            }
        }
        if !exempt {
            wall = nearby.map(|mirror| column.surface - mirror >= 4);
        }
    }
    Cell {
        wall,
        lake: water
            && matches!(
                kind,
                SurfaceKind::Lake | SurfaceKind::InlandSea | SurfaceKind::Swamp
            ),
        river: kind == SurfaceKind::River,
        water,
        level,
        dry: dry as f32,
        zone,
    }
}

/// §10.1: equal-distance samples of the meandered centerline. Grid columns
/// weight a broad channel more heavily than a narrow one; they are reported
/// separately for comparison with the historical VO0 reports.
#[derive(Default)]
pub struct AxisStats {
    pub points: u64,
    pub above_sea: u64,
    pub wet: u64,
    pub gorge: [f64; 3],
}

pub fn river_axis(
    generator: &Generator,
    center: (f64, f64),
    window: i64,
) -> Result<AxisStats, String> {
    let params = generator.params();
    let l = params.block.l_blocks;
    let half = window as f64 / 2.0;
    let step = if params.mode == litho_core::Mode::Gameplay {
        8.0
    } else {
        16.0
    };
    let shift = generator.spawn_shift();
    let center_l = generator.macro_point(center.0, center.1);
    let radius = half * 2.0_f64.sqrt() / l + litho_core::channel::max_query_radius_l(params);
    let mut stats = AxisStats::default();
    let mut depths = Vec::new();
    for plate in litho_core::plates::plates_near(center_l, radius, generator.seed(), params) {
        let net = match generator.drainage_network(&plate) {
            Ok(net) => net,
            Err(litho_core::drainage::RiverNetError::NoOwnedNode { .. }) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let channels = net.channels();
        for reach in 0..channels.reaches().len() {
            let mut along = 0.0;
            let mut next = step / 2.0;
            for segment in channels.reach_segments(reach) {
                let a = segment.a.point.sub(shift).scale(l);
                let delta = segment.b.point.sub(segment.a.point).scale(l);
                let length = delta.len();
                // Carry the sample phase across segment boundaries; a finely
                // subdivided bend must not earn more samples than a straight.
                while next < along + length {
                    let point = a.add(delta.scale((next - along) / length));
                    next += step;
                    if (point.x - center.0).abs() > half || (point.z - center.1).abs() > half {
                        continue;
                    }
                    let s0 = generator.s0_node(point.x, point.z).ok_or("нет S0 на оси")?;
                    if !is_land(s0.macro_y) {
                        continue;
                    }
                    let column = generator.water_column(point.x, point.z);
                    stats.points += 1;
                    if column.has_water() {
                        stats.above_sea += 1;
                    }
                    if column.has_water() || column.has_sea_river() {
                        stats.wet += 1;
                        depths.push(s0.macro_y - f64::from(column.level));
                    }
                }
                along += length;
            }
        }
    }
    depths.sort_by(f64::total_cmp);
    stats.gorge = [0.50, 0.90, 0.99].map(|q| percentile(&depths, q));
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossing_body_counts_once_but_area_is_split_by_zone() {
        let mut cells = vec![
            Cell {
                dry: 70.0,
                zone: 1,
                ..Cell::default()
            };
            4
        ];
        for cell in &mut cells[..3] {
            cell.water = true;
            cell.lake = true;
            cell.level = 80;
        }
        cells[0].zone = 0;
        let w = Window {
            width: 4,
            height: 1,
            step: 8,
            l_blocks: 256.0,
            w: true,
            cells,
        };
        let stats = w.stats(false);
        assert_eq!(stats.lakes, 1);
        assert_eq!(stats.zone_lakes[..2], [0, 1]);
        assert_eq!(stats.zone_land[..2], [1, 3]);
        assert_eq!(stats.zone_area_pct[0], 100.0);
        assert_eq!(stats.zone_area_pct[1], 200.0 / 3.0);
        assert_eq!(stats.lakes_per_100l2, 25600.0);
        assert_eq!(stats.flooded_land_pct, 75.0);
        assert_eq!(stats.lake_fit_pct, 100.0);
        assert_eq!(w.stats(true).lakes, 0); // 192 < 256 blocks²
    }
}
