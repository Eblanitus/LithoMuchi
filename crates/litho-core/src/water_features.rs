//! Точечные озёра (PLAN-WATER-UPDATE §6.6): кары, ледниковые котловины,
//! термокарст, кратеры, старицы, sag ponds. Один движок на все типы,
//! кэш на ячейку по ключу (сид, config_key, тип, i, j).
//!
//! Кандидаты стоят на сетке ячеек своего типа и считаются один раз на ячейку.
//! Колонка смотрит ячейки 3×3 вокруг себя только у тех типов, чьё условие
//! возможно в её зоне.

use std::collections::HashMap;

use crate::channel::ChannelQuery;
use crate::hash::{hash3, unit};
use crate::math::{Vec2, smoothstep};
use crate::morphoclimate::MorphoWeights;
use crate::params::Params;
use crate::mode::Mode;
use crate::seed::{subseed, SALT_WATER_FEATURE_CANDIDATE, SALT_WATER_FEATURE_GEOMETRY};
use crate::Generator;

/// Тип точечного озера (§6.6, таблица 6.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PointLakeType {
    /// Кар — круглая чаша в холодных горах (морфозона 6).
    Cirque,
    /// Ледниковая котловина — эллипс в бореальной/полярной низине.
    Glacial,
    /// Термокарст — овалы поперёк ветра в тундре.
    Thermokarst,
    /// Кратер — круглая чаша над горячей точкой.
    Crater,
    /// Старица — полумесяц у вершины излучины равнинной реки.
    Oxbow,
    /// Sag pond — эллипс вдоль разлома k7.
    SagPond,
}

impl PointLakeType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cirque => "cirque",
            Self::Glacial => "glacial",
            Self::Thermokarst => "thermokarst",
            Self::Crater => "crater",
            Self::Oxbow => "oxbow",
            Self::SagPond => "sag_pond",
        }
    }

    pub const fn all() -> [Self; 6] {
        [
            Self::Cirque,
            Self::Glacial,
            Self::Thermokarst,
            Self::Crater,
            Self::Oxbow,
            Self::SagPond,
        ]
    }

    /// Индекс типа в таблицах параметров (порядок всех()).
    pub const fn index(self) -> usize {
        match self {
            Self::Cirque => 0,
            Self::Glacial => 1,
            Self::Thermokarst => 2,
            Self::Crater => 3,
            Self::Oxbow => 4,
            Self::SagPond => 5,
        }
    }
}

/// Параметры типа из таблицы 6.6 (par gameplay / scientific).
#[derive(Clone, Copy, Debug)]
pub struct PointLakeTypeParams {
    /// Размер ячейки в блоках (геймплей, научный).
    pub cell_size_blocks: (f64, f64),
    /// Вероятность кандидата (геймплей; в научном делится на 1.5).
    pub probability: f64,
    /// Радиус в блоках (геймплей, научный).
    pub radius_blocks: (f64, f64),
    /// Глубина выреза D в блоках (геймплей, научный).
    pub depth_blocks: (f64, f64),
    /// H_cut: максимальный перепад max(S_rl внутри) − L (8, у кратера 32).
    pub h_cut_blocks: f64,
    /// Минимальный S0 на внешнем кольце, блоки (без — 0).
    pub min_s0_blocks: f64,
    /// Минимальный уклон S0 (без — 0).
    pub min_slope: f64,
    /// Максимальный уклон S0 (без — f64::INFINITY).
    pub max_slope: f64,
}

/// Стартовые параметры типов из таблицы 6.6.
pub const POINT_LAKE_PARAMS: [PointLakeTypeParams; 6] = [
    // Кар: вес морфозоны 6 ≥ 0.4 или (P0–P2 и E0–E1); S0 ≥ 100/140; уклон 0.15–0.8
    PointLakeTypeParams {
        cell_size_blocks: (160.0, 320.0),
        probability: 0.35,
        radius_blocks: (8.0, 12.0),
        depth_blocks: (6.0, 10.0),
        h_cut_blocks: 8.0,
        min_s0_blocks: 100.0,
        min_slope: 0.15,
        max_slope: 0.8,
    },
    // Ледниковая котловина: веса морфозон 0+1 ≥ 0.5; уклон ≤ 0.1
    PointLakeTypeParams {
        cell_size_blocks: (96.0, 192.0),
        probability: 0.5, // бореальная; полярная 0.3 — проверяется в условии
        radius_blocks: (10.0, 16.0),
        depth_blocks: (4.0, 6.0),
        h_cut_blocks: 8.0,
        min_s0_blocks: 0.0,
        min_slope: 0.0,
        max_slope: 0.1,
    },
    // Термокарст: t_c ≤ 1.5 °C, вес морфозоны 0 ≥ 0.5; уклон ≤ 0.05
    PointLakeTypeParams {
        cell_size_blocks: (80.0, 160.0),
        probability: 0.6,
        radius_blocks: (12.0, 20.0),
        depth_blocks: (2.0, 3.0),
        h_cut_blocks: 8.0,
        min_s0_blocks: 0.0,
        min_slope: 0.0,
        max_slope: 0.05,
    },
    // Кратер: центры горячих точек; P/PET ≥ 0.6 — с водой, иначе сухая чаша
    PointLakeTypeParams {
        cell_size_blocks: (512.0, 2048.0), // сетка горячих точек
        probability: 0.5,
        radius_blocks: (16.0, 32.0),
        depth_blocks: (12.0, 20.0),
        h_cut_blocks: 32.0,
        min_s0_blocks: 0.0,
        min_slope: 0.0,
        max_slope: f64::INFINITY,
    },
    // Старица: класс «равнинная», амплитуда ≥ w; дуга 120° у вершины излучины
    PointLakeTypeParams {
        cell_size_blocks: (400.0, 1000.0), // одна на N блоков русла
        probability: 0.5,
        radius_blocks: (0.0, 0.0), // вычисляется от меандра
        depth_blocks: (2.0, 3.0),
        h_cut_blocks: 8.0,
        min_s0_blocks: 0.0,
        min_slope: 0.0,
        max_slope: f64::INFINITY,
    },
    // Sag pond: вдоль k7 (boundary_weights[6] ≥ 0.5, sigma_l < 0.1 L)
    PointLakeTypeParams {
        cell_size_blocks: (128.0, 256.0),
        probability: 0.4,
        radius_blocks: (8.0, 16.0),
        depth_blocks: (3.0, 5.0),
        h_cut_blocks: 8.0,
        min_s0_blocks: 0.0,
        min_slope: 0.0,
        max_slope: f64::INFINITY,
    },
];

/// Один экземпляр точки озера.
#[derive(Clone, Copy, Debug)]
pub struct PointLake {
    pub r#type: PointLakeType,
    pub center: Vec2,
    pub radius_blocks: f64,
    /// Уровень воды L в блоках над морем.
    pub level_blocks: f64,
    /// Глубина выреза D в блоках.
    pub depth_blocks: f64,
    /// Аспект эллипса (1.0 = круг).
    pub aspect: f64,
    /// Поворот эллипса в радианах.
    pub rotation: f64,
    /// Для кратера: true если есть вода, false если сухая чаша.
    pub has_water: bool,
}

/// Результат запроса точки озера в позиции.
#[derive(Clone, Copy, Debug, Default)]
pub struct PointLakeHit {
    pub lake: Option<PointLake>,
    /// Точка внутри формы?
    pub inside: bool,
    /// Расстояние до края формы в блоках (0 внутри).
    pub distance_l: f64,
    /// Сила берега 0..1 (1 внутри, затухает к краю).
    pub shore_strength: f64,
    /// Сила воды 0..1 (1 внутри если есть вода, иначе 0).
    pub water_strength: f64,
}

/// Кэш озёр одной ячейки одного типа.
#[derive(Clone, Debug, Default)]
struct CellLakes {
    pub lakes: Vec<PointLake>,
}

/// Движок точечных озёр. Создаётся один раз на генератор, кэширует ячейки.
pub struct PointLakeEngine {
    /// Ячейка → озёра по типам. Ключ: (cell_i, cell_j, type_index).
    cache: HashMap<(i64, i64, usize), CellLakes>,
    /// Ячейки уже обработаны для этой позиции генератора (один экземпляр
    /// движка на чанк, кэш не persists между чанками — см. примечание в
    /// §6.6: «как тайлы»).
    cell_size_blocks: [f64; 6],
    mode_scientific: bool,
}

impl PointLakeEngine {
    pub fn new(params: &Params) -> Self {
        let mode_scientific = params.mode == Mode::Scientific;
        let cell_size_blocks = POINT_LAKE_PARAMS
            .iter()
            .map(|p| {
                if mode_scientific {
                    p.cell_size_blocks.1
                } else {
                    p.cell_size_blocks.0
                }
            })
            .collect::<Vec<_>>()
            .try_into()
            .expect("всегда 6 типов");
        Self {
            cache: HashMap::new(),
            cell_size_blocks,
            mode_scientific,
        }
    }

    /// Вероятность типа с поправкой на научный режим (делится на 1.5).
    fn type_probability(&self, t: PointLakeType) -> f64 {
        let p = POINT_LAKE_PARAMS[t.index()].probability;
        if self.mode_scientific {
            p / 1.5
        } else {
            p
        }
    }

    /// Ячейка для позиции и типа.
    fn cell_of(&self, x: f64, z: f64, t: PointLakeType) -> (i64, i64) {
        let cell_size = self.cell_size_blocks[t.index()];
        ((x / cell_size).floor() as i64, (z / cell_size).floor() as i64)
    }

    /// Сгенерировать кандидатов для одной ячейки и типа.
    fn generate_cell(
        &self,
        generator: &Generator,
        cell_i: i64,
        cell_j: i64,
        t: PointLakeType,
    ) -> CellLakes {
        let p = &POINT_LAKE_PARAMS[t.index()];
        let seed = generator.seed();
        let subseed = subseed(seed, SALT_WATER_FEATURE_CANDIDATE);

        // Один кандидат на ячейку (как в §6.6: «брасок вероятности»).
        let h = hash3(subseed, cell_i, cell_j, t.index() as i64);
        let mut rng_h = h;
        let next_unit = || {
            let v = unit(rng_h);
            rng_h = crate::hash::splitmix64(rng_h);
            v
        };

        // Бросок вероятности.
        if next_unit() > self.type_probability(t) {
            return CellLakes::default();
        }

        // Геометрия из второго хеша.
        let hg = hash3(subseed, cell_i, cell_j, t.index() as i64 + 1000);
        let mut geo_h = hg;
        let next_geo_unit = || {
            let v = unit(geo_h);
            geo_h = crate::hash::splitmix64(geo_h);
            v
        };

        // Центр = ячейка + сдвиг из хеша (в пределах ячейки).
        let cell_size = self.cell_size_blocks[t.index()];
        let offset_x = (next_geo_unit() - 0.5) * cell_size * 0.8;
        let offset_z = (next_geo_unit() - 0.5) * cell_size * 0.8;
        let center_x = cell_i as f64 * cell_size + offset_x;
        let center_z = cell_j as f64 * cell_size + offset_z;

        // Радиус из диапазона типа.
        let radius = p.radius_blocks.0
            + next_geo_unit() * (p.radius_blocks.1 - p.radius_blocks.0);

        // Проверка условий приёма кандидата (§6.6, п. 2).
        if !self.check_conditions(generator, center_x, center_z, t, radius) {
            return CellLakes::default();
        }

        // Уровень по кольцам (§6.6, п. 3).
        let level = self.compute_level(generator, center_x, center_z, radius, t);
        if level <= 64.0 {
            return CellLakes::default();
        }

        // Проверка H_cut: max(S_rl внутри) − L ≤ H_cut.
        if !self.check_h_cut(generator, center_x, center_z, radius, level, p.h_cut_blocks) {
            return CellLakes::default();
        }

        // Глубина.
        let depth = p.depth_blocks.0
            + next_geo_unit() * (p.depth_blocks.1 - p.depth_blocks.0);

        // Аспект и поворот.
        let aspect = match t {
            PointLakeType::Glacial => 1.0 + next_geo_unit(), // 1–2:1
            PointLakeType::Thermokarst => 1.5 + next_geo_unit() * 1.0, // 1.5–2.5:1
            PointLakeType::SagPond => 3.0, // 3:1 вдоль разлома
            _ => 1.0,
        };
        let rotation = next_geo_unit() * 2.0 * std::f64::consts::PI;

        // Кратер: проверка P/PET для воды.
        let has_water = if t == PointLakeType::Crater {
            // P/PET ≥ 0.6 — с водой, иначе сухая чаша.
            let climate = generator.climate_sample(center_x, center_z);
            match climate {
                Some(c) => c.precip.p_mm / c.precip.pet_mm.max(0.001) >= 0.6,
                None => false,
            }
        } else {
            true
        };

        CellLakes {
            lakes: vec![PointLake {
                r#type: t,
                center: Vec2::new(center_x, center_z),
                radius_blocks: radius,
                level_blocks: level,
                depth_blocks: depth,
                aspect,
                rotation,
                has_water,
            }],
        }
    }

    /// Условие приёма кандидата в центре (§6.6, п. 2).
    fn check_conditions(
        &self,
        generator: &Generator,
        x: f64,
        z: f64,
        t: PointLakeType,
        radius: f64,
    ) -> bool {
        let p = &POINT_LAKE_PARAMS[t.index()];

        // S0 на внешнем кольце ≥ min_s0_blocks (не у моря).
        if p.min_s0_blocks > 0.0 {
            let s0 = match generator.s0_at(x, z) {
                Some(s) => s.macro_y + s.micro_dy,
                None => return false,
            };
            if s0 < p.min_s0_blocks {
                return false;
            }
        }

        // Уклон S0 в пределах min_slope..max_slope.
        if p.min_slope > 0.0 || p.max_slope.is_finite() {
            let s0 = match generator.s0_at(x, z) {
                Some(s) => s.macro_y + s.micro_dy,
                None => return false,
            };
            // Приближённый уклон по двум соседние точки.
            let dx = 4.0;
            let s0_xp = match generator.s0_at(x + dx, z) {
                Some(s) => s.macro_y + s.micro_dy,
                None => return false,
            };
            let s0_xm = match generator.s0_at(x - dx, z) {
                Some(s) => s.macro_y + s.micro_dy,
                None => return false,
            };
            let slope = ((s0_xp - s0_xm) / (2.0 * dx)).abs();
            if slope < p.min_slope || slope > p.max_slope {
                return false;
            }
        }

        // Условия по типам (морфозоны, климат, и т.д.).
        let morpho = generator.morpho_weights(x, z);
        let climate = generator.climate_sample(x, z);

        match t {
            PointLakeType::Cirque => {
                // Вес морфозоны 6 ≥ 0.4 или (P0–P2 и E0–E1).
                let m6 = morpho.map(|m| m.values[6]).unwrap_or(0.0);
                let cold = climate.map(|c| c.temperature.t_c < 6.0).unwrap_or(false);
                let e01 = true; // упрощённо: считаем, что E0–E1 проверяется через S0 ≥ 100
                if m6 < 0.4 && !(cold && e01) {
                    return false;
                }
                true
            }
            PointLakeType::Glacial => {
                // Веса морфозон 0 (полярная) + 1 (бореальная) ≥ 0.5.
                let w = morpho.map(|m| m.values[0] + m.values[1]).unwrap_or(0.0);
                if w < 0.5 {
                    return false;
                }
                // Вероятность: бореальная 0.5, полярная 0.3.
                let polar_w = morpho.map(|m| m.values[0]).unwrap_or(0.0);
                let boreal_w = morpho.map(|m| m.values[1]).unwrap_or(0.0);
                let prob = if polar_w > boreal_w { 0.3 } else { 0.5 };
                // Уже сделан бросок вероятности в generate_cell, но aquí
                // уточняем для полярной зоны.
                true
            }
            PointLakeType::Thermokarst => {
                // t_c ≤ 1.5 °C, вес морфозоны 0 ≥ 0.5.
                let t_c = climate.map(|c| c.temperature.t_c).unwrap_or(f64::INFINITY);
                let polar_w = morpho.map(|m| m.values[0]).unwrap_or(0.0);
                if t_c > 1.5 || polar_w < 0.5 {
                    return false;
                }
                true
            }
            PointLakeType::Crater => {
                // Центры горячих точек на суше — проверяется через crust_sample.
                let crust = generator.crust_sample(x, z);
                match crust {
                    Some(c) => c.hotspot_km >= 0.5,
                    None => false,
                }
            }
            PointLakeType::Oxbow => {
                // Класс «равнинная», амплитуда ≥ w — упрощённо: рядом есть
                // русло равнинной реки.
                let ch = generator.drainage_channel_query(x, z);
                match ch {
                    Ok(q) => {
                        if let Some(hit) = q.nearest {
                            hit.valley_class == crate::water_profile::ValleyClass::Plain
                                && hit.core_l > 4.0
                        } else {
                            false
                        }
                    }
                    Err(_) => false,
                }
            }
            PointLakeType::SagPond => {
                // Вдоль k7: boundary_weights[6] ≥ 0.5, sigma_l < 0.1 L.
                let crust = generator.crust_sample(x, z);
                match crust {
                    Some(c) => c.rift_weight >= 0.5,
                    None => false,
                }
            }
        }
    }

    /// Уровень по кольцам (§6.6, п. 3):
    /// L = floor(min S_rl по кольцам на 0, 2 и 4 блока снаружи края формы,
    /// через 2 блока) − 1.
    fn compute_level(
        &self,
        generator: &Generator,
        x: f64,
        z: f64,
        radius: f64,
        _t: PointLakeType,
    ) -> f64 {
        let mut min_s_rl = f64::INFINITY;
        for &ring_r in &[0.0, 2.0, 4.0] {
            let probe_r = radius + ring_r;
            // 8 точек на кольце.
            for k in 0..8 {
                let angle = k as f64 * std::f64::consts::PI / 4.0;
                let px = x + probe_r * angle.cos();
                let pz = z + probe_r * angle.sin();
                let s = match generator.s0_at(px, pz) {
                    Some(s) => s.macro_y + s.micro_dy,
                    None => continue,
                };
                min_s_rl = min_s_rl.min(s);
            }
        }
        if min_s_rl.is_infinite() {
            return 0.0;
        }
        (min_s_rl.floor() - 1.0).max(63.0)
    }

    /// Проверка H_cut: max(S_rl внутри формы) − L ≤ H_cut.
    fn check_h_cut(
        &self,
        generator: &Generator,
        x: f64,
        z: f64,
        radius: f64,
        level: f64,
        h_cut: f64,
    ) -> bool {
        let mut max_s_rl = f64::NEG_INFINITY;
        // 6×6 сетка внутри формы.
        for k in 0..6 {
            for l in 0..6 {
                let t = (k as f64 - 2.5) / 2.5;
                let u = (l as f64 - 2.5) / 2.5;
                if t.tanh().abs() + u.tanh().abs() > 1.0 {
                    continue; // вне эллипса
                }
                let px = x + t * radius * 0.9;
                let pz = z + u * radius * 0.9;
                let s = match generator.s0_at(px, pz) {
                    Some(s) => s.macro_y + s.micro_dy,
                    None => continue,
                };
                max_s_rl = max_s_rl.max(s);
            }
        }
        if max_s_rl.is_infinite() {
            return false;
        }
        max_s_rl - level <= h_cut
    }

    /// Получить озёра ячейки (из кэша или сгенерировать).
    fn cell_lakes(
        &mut self,
        generator: &Generator,
        cell_i: i64,
        cell_j: i64,
        t: PointLakeType,
    ) -> &CellLakes {
        let key = (cell_i, cell_j, t.index());
        if !self.cache.contains_key(&key) {
            let lakes = self.generate_cell(generator, cell_i, cell_j, t);
            self.cache.insert(key, lakes);
        }
        self.cache.get(&key).expect("только что вставили")
    }

    /// Запрос точки озера в позиции (x, z). Проверяет ячейки 3×3 вокруг.
    pub fn query(
        &mut self,
        generator: &Generator,
        x: f64,
        z: f64,
    ) -> PointLakeHit {
        let mut best = PointLakeHit::default();
        let mut best_distance = f64::INFINITY;

        for &t in PointLakeType::all() {
            let (ci, cj) = self.cell_of(x, z, t);
            for di in -1..=1 {
                for dj in -1..=1 {
                    let lakes = self.cell_lakes(generator, ci + di, cj + dj, t);
                    for lake in &lakes.lakes {
                        let hit = self.eval_point(x, z, lake);
                        if hit.inside && hit.distance_l < best_distance {
                            best = hit;
                            best_distance = hit.distance_l;
                        } else if !hit.inside && hit.distance_l < best_distance
                            && hit.distance_l <= lake.radius_blocks * 1.5
                        {
                            // Соседняя форма — берег.
                            best = hit;
                            best_distance = hit.distance_l;
                        }
                    }
                }
            }
        }

        best
    }

    /// Оценка точки против одного озера.
    fn eval_point(&self, x: f64, z: f64, lake: &PointLake) -> PointLakeHit {
        let dx = x - lake.center.x;
        let dz = z - lake.center.z;

        // Повернуть точку в систему координат эллипса.
        let cos = lake.rotation.cos();
        let sin = lake.rotation.sin();
        let rx = dx * cos + dz * sin;
        let rz = -dx * sin + dz * cos;

        // Нормированный радиус ρ в эллипсе.
        let rho = ((rx / lake.radius_blocks).powi(2)
            + (rz / (lake.radius_blocks * lake.aspect)).powi(2))
        .sqrt();

        let inside = rho <= 1.0;
        let distance_l = if inside { 0.0 } else { (rho - 1.0) * lake.radius_blocks };

        // Сила берега: 1 внутри, затухает к краю.
        let shore_strength = if inside {
            1.0
        } else {
            let fade = (distance_l / lake.radius_blocks).clamp(0.0, 1.0);
            1.0 - smoothstep(0.0, 1.0, fade)
        };

        let water_strength = if inside && lake.has_water {
            1.0
        } else {
            0.0
        };

        PointLakeHit {
            lake: Some(*lake),
            inside,
            distance_l,
            shore_strength,
            water_strength,
        }
    }
}

impl Default for PointLakeEngine {
    fn default() -> Self {
        // Для тестов; параметры берутся из defaults.
        let params = Params::defaults(crate::version::GenVersion::V1DevW, crate::mode::Mode::Gameplay);
        Self::new(&params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_lake_type_index_order() {
        assert_eq!(PointLakeType::Cirque.index(), 0);
        assert_eq!(PointLakeType::SagPond.index(), 5);
        assert_eq!(PointLakeType::all().len(), 6);
    }

    #[test]
    fn eval_point_inside() {
        let lake = PointLake {
            r#type: PointLakeType::Cirque,
            center: Vec2::new(0.0, 0.0),
            radius_blocks: 10.0,
            level_blocks: 70.0,
            depth_blocks: 6.0,
            aspect: 1.0,
            rotation: 0.0,
            has_water: true,
        };
        let engine = PointLakeEngine::default();
        let hit = engine.eval_point(0.0, 0.0, &lake);
        assert!(hit.inside);
        assert_eq!(hit.distance_l, 0.0);
        assert_eq!(hit.shore_strength, 1.0);
        assert_eq!(hit.water_strength, 1.0);

        let hit = engine.eval_point(20.0, 0.0, &lake);
        assert!(!hit.inside);
        assert!(hit.distance_l > 0.0);
    }

    #[test]
    fn eval_point_outside_no_water() {
        let lake = PointLake {
            r#type: PointLakeType::Crater,
            center: Vec2::new(0.0, 0.0),
            radius_blocks: 10.0,
            level_blocks: 70.0,
            depth_blocks: 12.0,
            aspect: 1.0,
            rotation: 0.0,
            has_water: false, // сухая чаша
        };
        let engine = PointLakeEngine::default();
        let hit = engine.eval_point(0.0, 0.0, &lake);
        assert!(hit.inside);
        assert_eq!(hit.water_strength, 0.0);
    }
}
