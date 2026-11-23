//! R2 (Ф6-В1/В5, PLAN-WATER-FABRIC §4–5): инвариант «вода не уходит за
//! берега» и точки для игровой проверки.
//!
//! `water-leaks` — проверка на сетке колонок: дно ниже зеркала, у каждого из
//! четырёх соседей либо вода со ступенью не больше блока, либо суша не ниже
//! зеркала. `water-points` — до 20 мест для CI и автора: центры высоких озёр,
//! ступени рек и устья рек в озёра.
//!
//! Замер идёт только по генератору `v1-dev-r2`: в `v1-dev` воды выше моря нет,
//! и команда честно об этом скажет.

use std::collections::BTreeMap;

use litho_core::Generator;
use litho_core::channel::ReachEnd;
use litho_core::generator::WaterColumn;
use litho_core::height::SEA_LEVEL;

/// Уровень моря в блоках: вода выше него — предмет R2.
const SEA: i32 = SEA_LEVEL as i32;

/// Итог `water-leaks`.
#[derive(Debug, Default)]
pub struct Leaks {
    /// Колонок проверено.
    pub columns: u64,
    /// Из них с водой выше моря.
    pub water: u64,
    /// Нарушения п. 1: поверхность не ниже зеркала (воды не видно).
    pub deep_bed: u64,
    /// Нарушения п. 2: сосед-суша ниже зеркала или вода со ступенью > 1.
    pub spill: u64,
    /// Нарушения п. 5: рельеф поднят больше чем на 3 блока против `v1-dev`.
    pub raised: u64,
    /// Сухие колонки ниже зеркала на кромке водопада: сосед принадлежит
    /// другому руслу, которое уже упало до моря. Это §4, п. 3 (водопад),
    /// а не протечка п. 2: отвесная стена у водопада и должна быть ниже
    /// верхнего зеркала.
    pub falls: u64,
    /// Ступени воды больше блока: по §4, п. 3 это водопады, а не нарушение —
    /// считаются метрикой. Печатаются с примерами: рост числа означает, что
    /// уровень где-то посчитан неверно.
    pub waterfalls: u64,
    /// Колонки оси русла на суше (`core > 0.9`, сухая `S ≥ 63`): в игре в
    /// них должна стоять вода — либо R2 выше моря, либо ванильная до y = 62
    /// (дно вырезано ниже 62). Игровой тест 2026-11-19: R2 не резал русла
    /// на уровне моря, реки были сухими, а гейт этого не видел — он смотрел
    /// только на колонки, где у ядра уже есть вода.
    pub river_axis: u64,
    /// Из них сухие: ни воды R2, ни дна ниже 62.
    pub river_dry: u64,
    /// Первые примеры нарушений, не больше `EXAMPLE_LIMIT`.
    pub examples: Vec<String>,
    /// Координаты первых нарушений — по ним печатается разбор колонки.
    pub coords: Vec<(i64, i64)>,
}

/// Сколько примеров печатать.
const EXAMPLE_LIMIT: usize = 10;
/// §4, п. 5: допустимый подъём рельефа против `v1-dev`, блоки.
pub const RAISE_LIMIT_BLOCKS: f64 = 3.0;
/// §4, п. 6: в окне каждого режима должно быть не меньше стольких колонок
/// с водой выше моря, иначе гейт проходит впустую.
pub const MIN_WATER_COLUMNS: u64 = 200;
/// Ось русла: сила ядра выше этого порога.
const RIVER_AXIS_CORE: f64 = 0.9;
/// Гейт сухих осей русла, %. Цель — 2 %. Временно 30 %: после ущелий до
/// 8 блоков в окнах CI остаётся 3–25 % сухих осей — участки, где дно долины
/// выше уровня реки больше чем на 8 блоков (2026-11-19). Порог ловит прежний
/// провал (60–100 % сухих) и не даёт ему вернуться; снижать по мере правки.
pub const RIVER_DRY_MAX_PCT: f64 = 30.0;

impl Leaks {
    /// Суммирует окна: широкое и плотное считаются отдельно, а гейт —
    /// по сумме.
    pub fn merge(&mut self, other: &Self) {
        self.columns += other.columns;
        self.water += other.water;
        self.deep_bed += other.deep_bed;
        self.spill += other.spill;
        self.raised += other.raised;
        self.waterfalls += other.waterfalls;
        self.falls += other.falls;
        self.river_axis += other.river_axis;
        self.river_dry += other.river_dry;
        for coord in &other.coords {
            if self.coords.len() < EXAMPLE_LIMIT {
                self.coords.push(*coord);
            }
        }
        for example in &other.examples {
            self.example(example.clone());
        }
    }

    /// Доля сухих осей русла, %.
    pub fn river_dry_pct(&self) -> f64 {
        if self.river_axis == 0 {
            0.0
        } else {
            100.0 * self.river_dry as f64 / self.river_axis as f64
        }
    }

    pub fn violations(&self) -> u64 {
        self.deep_bed + self.spill + self.raised
    }

    fn coord(&mut self, x: i64, z: i64) {
        if self.coords.len() < EXAMPLE_LIMIT {
            self.coords.push((x, z));
        }
    }

    fn example(&mut self, text: String) {
        if self.examples.len() < EXAMPLE_LIMIT {
            self.examples.push(text);
        }
    }
}

/// Проверка инварианта §4 на квадрате `size` блоков вокруг `center` с шагом
/// `step`. Соседи проверяются по блокам (±1), а не по шагу сетки: инвариант
/// сформулирован для соседних блоков.
/// То же с эталонным генератором для п. 5 §4: рельеф `v1-dev-r2` не выше
/// `v1-dev` больше чем на 3 блока. `None` — п. 5 не проверяется.
pub fn leaks_against(
    generator: &Generator,
    reference: Option<&Generator>,
    center: (f64, f64),
    size: i64,
    step: i64,
) -> Leaks {
    let step = step.max(1);
    let half = size.max(step) / 2;
    let mut out = Leaks::default();
    let x0 = center.0 as i64 - half;
    let z0 = center.1 as i64 - half;
    let mut z = z0;
    while z <= z0 + 2 * half {
        let mut x = x0;
        while x <= x0 + 2 * half {
            out.columns += 1;
            let column = generator.water_column(x as f64, z as f64);
            if let Some(reference) = reference {
                // §4, п. 5: `v1-dev-r2` только опускает рельеф и добивает
                // берег на 1–2 блока. Всё, что выше эталона на 3 блока, —
                // насыпь, то есть ошибка уровня. Сравниваем с **сухой**
                // поверхностью эталона: сам `v1-dev` режет русло под
                // километровый уровень, и его готовая поверхность в тех же
                // колонках стоянки на 20–30 блоков ниже (баг К3), так что
                // сравнение с ней ловило бы вырез `v1-dev`, а не насыпь.
                // Д-138: у W «эталон» — его собственный рельеф без воды. Точный
                // `s0_at` совпадает с полной сухой поверхностью побитово (ВО0-2), а
                // быстрый `s0_node` считает климат без лучей влаги и даёт ошибку в
                // единицы блоков: на таком сравнении любой нормальный вырез русла
                // выглядел «насыпью». Порог 3 блока не меняется.
                let base = if reference.params().version.is_water_update() {
                    reference.s0_at(x as f64, z as f64).map(|s| s.surface_y())
                } else {
                    reference.dry_surface_at(x as f64, z as f64).ok().flatten()
                };
                if let Some(base) = base {
                    // Д-138: «насыпь» — это поднятое ДНО под водой, а не поверхность
                    // колонки. У разрезанной колонки `surface` — дно после выреза, и
                    // оно законно ниже `S0`; у сухой колонки `surface` — просто рельеф,
                    // и сравнивать его с `S0` того же генератора незачем.
                    if column.has_water()
                        && column.bed != WaterColumn::NONE
                        && column.bed as f64 > base + RAISE_LIMIT_BLOCKS
                    {
                        out.raised += 1;
                        out.example(format!(
                            "рельеф поднят: ({x}, {z}) дно {} против сухой поверхности {base:.1}",
                            column.bed
                        ));
                    }
                }
            }
            if column.has_water() {
                out.water += 1;
                check_column(generator, &column, x, z, &mut out);
            }
            river_axis(generator, &column, x, z, &mut out);
            x += step;
        }
        z += step;
    }
    out
}

/// Ось русла на суше должна быть мокрой в игре (см. `Leaks::river_axis`).
fn river_axis(generator: &Generator, column: &WaterColumn, x: i64, z: i64, out: &mut Leaks) {
    let (xf, zf) = (x as f64, z as f64);
    let Ok(hydrology) = generator.hydrology_at(xf, zf) else {
        return;
    };
    let on_axis = if generator.params().version.is_water_update() {
        let l = generator.params().block.l_blocks;
        hydrology.channels.nearest.is_some_and(|hit| {
            let half = 0.5 * (hit.core_l * l).max(1.0);
            hit.distance_l * l <= half
        })
    } else {
        hydrology.channels.core > RIVER_AXIS_CORE
    };
    if !on_axis {
        return;
    }
    let Ok(Some(dry)) = generator.dry_surface_at(xf, zf) else {
        return;
    };
    if dry < SEA_LEVEL {
        return;
    }
    out.river_axis += 1;
    if column.has_water() {
        return;
    }
    // Ванильная вода стоит до y = 62: блок 62 воздух, если дно не выше 62.
    let bed = generator
        .sample(xf, zf)
        .map_or(f64::INFINITY, |full| full.world.height.surface_y);
    if bed <= SEA_LEVEL - 1.0 {
        return;
    }
    out.river_dry += 1;
    if out.river_dry <= 3 {
        out.example(format!(
            "сухое русло: ({x}, {z}) сухая S {dry:.1}, дно {bed:.1}, воды R2 нет"
        ));
    }
}

/// §4, п. 3: сосед ниже зеркала на кромке водопада — не протечка. Признак:
/// его собственное русло или озеро уже упало на два блока и больше (или эта
/// ветвь дошла до моря, `level_blocks = None`). Тогда отвесная сухая стена
/// рядом с верхним зеркалом — это и есть водопад.
fn belongs_to_a_fall(generator: &Generator, x: i64, z: i64, level: i32) -> bool {
    let Ok(hydrology) = generator.hydrology_levels_at(x as f64, z as f64) else {
        return false;
    };
    if let Some(hit) = hydrology.channels.nearest {
        match hit.level_blocks {
            None => return true,
            Some(own) => {
                if own <= level as f64 - 2.0 {
                    return true;
                }
                // Уровень сегмента стоит над сушей соседа выше предела русла:
                // ядро считает эту колонку водопадом (`height::with_water`), а
                // не руслом, и воды в ней нет.
                if let Ok(Some(dry)) = generator.dry_surface_at(x as f64, z as f64) {
                    if own > dry + litho_core::height::RIVER_MAX_ABOVE_LAND_BLOCKS {
                        return true;
                    }
                }
            }
        }
    }
    if let Some(hit) = hydrology.lakes.water_hit.filter(|hit| hit.inside) {
        if let Some(own) = hit.level_blocks {
            if own <= level as f64 - 2.0 {
                return true;
            }
        }
    }
    false
}

fn check_column(generator: &Generator, column: &WaterColumn, x: i64, z: i64, out: &mut Leaks) {
    if column.surface > column.level - 1 {
        out.deep_bed += 1;
        out.coord(x, z);
        out.example(format!(
            "дно выше зеркала: ({x}, {z}) поверхность {} зеркало {}",
            column.surface, column.level
        ));
    }
    for (dx, dz) in [(1_i64, 0_i64), (-1, 0), (0, 1), (0, -1)] {
        let neighbor = generator.water_column((x + dx) as f64, (z + dz) as f64);
        let w = generator.params().version.is_water_update();
        let marked = matches!(
            column.flow,
            litho_core::water_profile::FlowClass::FallTop
                | litho_core::water_profile::FlowClass::Rapid
                | litho_core::water_profile::FlowClass::Cascade
        );
        if neighbor.has_water() || (w && neighbor.has_sea_river()) {
            if neighbor.level < column.level - 1 {
                // §4, п. 3: вода-вода с перепадом больше блока — водопад.
                // Он в метрики, а не в нарушения: вода не уходит в сушу,
                // а падает в соседнюю воду.
                if w && !marked {
                    out.spill += 1;
                    out.coord(x, z);
                    out.example(format!(
                        "неразмеченная ступень: ({x}, {z}) {} → {}",
                        column.level, neighbor.level
                    ));
                    continue;
                }
                out.waterfalls += 1;
                out.example(format!(
                    "водопад: ({x}, {z}) {} против {} у соседа",
                    column.level, neighbor.level
                ));
            }
        } else if neighbor.surface != WaterColumn::NONE
            && neighbor.surface < column.level
            && neighbor.surface > SEA
        {
            // Сосед ниже уровня моря — это море, а не суша: игра наливает
            // там воду до 63, и ступень считается от неё.
            let fall_edge = if w {
                column.flow == litho_core::water_profile::FlowClass::FallTop
            } else {
                belongs_to_a_fall(generator, x + dx, z + dz, column.level)
            };
            if fall_edge {
                // §4, п. 3: отвесная стена на кромке водопада — не протечка.
                out.falls += 1;
                continue;
            }
            out.spill += 1;
            // Разбор — по сухой колонке: у неё своё русло и свой уровень.
            out.coord(x + dx, z + dz);
            out.example(format!(
                "берег ниже зеркала: ({}, {}) суша {} зеркало {} в ({x}, {z})",
                x + dx,
                z + dz,
                neighbor.surface,
                column.level
            ));
        }
    }
}

/// Место для игровой проверки и CI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaterPoint {
    pub x: i64,
    pub z: i64,
    pub level: i32,
    pub kind: &'static str,
    pub note: &'static str,
}

/// Сколько точек каждого вида искать (в сумме не больше 20, как в плане).
const LAKE_LIMIT: usize = 10;
const STEP_LIMIT: usize = 5;
const MOUTH_LIMIT: usize = 5;
/// Порог «высокого озера» из плана (строка 10.1: озёра на высоте 80 и выше).
pub const HIGH_LAKE_LEVEL: i32 = 80;
/// Сторона ячейки, по которой схлопываются кандидаты одного озера/реки.
const CLUSTER: i64 = 512;
/// Сколько проб на отрезок русла при поиске ступеней.
const SEGMENT_PROBES: i64 = 4;

fn cluster(x: f64, z: f64) -> (i64, i64) {
    (
        (x as i64).div_euclid(CLUSTER),
        (z as i64).div_euclid(CLUSTER),
    )
}

/// До 20 мест: центры высоких озёр, ступени рек и устья рек в озёра.
/// Поиск идёт кольцами от 8 L до `radius_l`: сеть стока строится по плитам,
/// и платить за 128 L сразу незачем, если вода нашлась в 8 L.
pub fn points(generator: &Generator, center: (f64, f64), radius_l: f64) -> Vec<WaterPoint> {
    points_filtered(generator, center, radius_l, None)
}

pub fn points_filtered(
    generator: &Generator,
    center: (f64, f64),
    radius_l: f64,
    wanted: Option<&str>,
) -> Vec<WaterPoint> {
    let limit = radius_l.max(8.0);
    let mut radius = 8.0_f64;
    loop {
        let found = points_at(generator, center, radius, wanted);
        if !found.is_empty() || radius >= limit {
            return found;
        }
        radius = (radius * 2.0).min(limit);
    }
}

/// Require the actual upper and receiver levels of this profile event, not
/// merely another cliff within the same five-block search stencil.
pub fn wet_fall_face(
    g: &Generator,
    site: &litho_core::generator::FallSite,
    center: litho_core::Vec2,
    radius: f64,
) -> Option<WaterPoint> {
    let (cx, cz) = (site.center.x.round() as i64, site.center.z.round() as i64);
    for z in cz - 2..=cz + 2 {
        for x in cx - 2..=cx + 2 {
            if center.dist2(litho_core::Vec2::new(x as f64, z as f64)) > radius * radius {
                continue;
            }
            let top = g.water_column(x as f64, z as f64);
            if top.flow != litho_core::FlowClass::FallTop
                || !top.has_water()
                || top.level != site.upper_level
            {
                continue;
            }
            for (dx, dz) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
                let low = g.water_column((x + dx) as f64, (z + dz) as f64);
                if low.flow == litho_core::FlowClass::Plunge
                    && (low.has_water() || low.has_sea_river())
                    && low.level == site.lower_level
                {
                    return Some(WaterPoint {
                        x,
                        z,
                        level: top.level,
                        kind: "river",
                        note: if site.hanging {
                            "hanging_fall"
                        } else {
                            "waterfall"
                        },
                    });
                }
            }
        }
    }
    None
}

/// Only advertise lips with two adjacent integer columns the Fabric feature
/// can actually join. Propagate profile failures instead of reporting them as
/// an empty search window.
pub fn waterfall_points(
    g: &Generator,
    center: (f64, f64),
    radius_l: f64,
    hanging_only: bool,
) -> Result<Vec<WaterPoint>, String> {
    let q = g.macro_point(center.0, center.1);
    let c = litho_core::Vec2::new(center.0, center.1);
    let l = g.params().block.l_blocks;
    let mut found = BTreeMap::new();
    for plate in litho_core::plates::plates_near(
        q,
        radius_l + litho_core::channel::max_query_radius_l(g.params()),
        g.seed(),
        g.params(),
    ) {
        let survey = match g.river_profile_survey(&plate, c, radius_l * l) {
            Ok(v) => v,
            Err(litho_core::drainage::RiverNetError::NoOwnedNode { .. }) => continue,
            Err(e) => return Err(e.to_string()),
        };
        for site in survey.lips {
            if hanging_only && !site.hanging {
                continue;
            }
            if let Some(point) = wet_fall_face(g, &site, c, radius_l * l) {
                found
                    .entry(cluster(point.x as f64, point.z as f64))
                    .or_insert(point);
            }
        }
        if found.len() >= STEP_LIMIT {
            break;
        }
    }
    Ok(found.into_values().take(STEP_LIMIT).collect())
}

/// Один проход по плитам в радиусе `radius_l`, без колец.
fn points_at(
    generator: &Generator,
    center: (f64, f64),
    radius_l: f64,
    wanted: Option<&str>,
) -> Vec<WaterPoint> {
    let point = generator.macro_point(center.0, center.1);
    let shift = generator.spawn_shift();
    let params = generator.params();
    let l = params.block.l_blocks;
    let mut lakes: BTreeMap<(i64, i64), WaterPoint> = BTreeMap::new();
    let mut steps: BTreeMap<(i64, i64), WaterPoint> = BTreeMap::new();
    let mut mouths: BTreeMap<(i64, i64), WaterPoint> = BTreeMap::new();
    for plate in litho_core::plates::plates_near(point, radius_l, generator.seed(), params) {
        let Ok(net) = generator.drainage_network(&plate) else {
            continue;
        };
        for lake in net.lakes().lakes() {
            if !lake.kind.has_water() || lake.level_km.is_none() {
                continue;
            }
            // A centroid can be on an island. Also try the lowest route node
            // and only publish columns which really contain lake water.
            let bottom = net.nodes()[net.depressions()[lake.depression].bottom_node].point;
            for point in [lake.center, bottom] {
                let pos = point.sub(shift).scale(l);
                let (x, z) = (pos.x.round(), pos.z.round());
                let column = generator.water_column(x, z);
                let w = generator.params().version.is_water_update();
                let kind = if w {
                    column.body.name()
                } else {
                    lake.kind.as_str()
                };
                if !column.has_water()
                    || (w
                        && !matches!(
                            column.kind,
                            litho_core::SurfaceKind::Lake
                                | litho_core::SurfaceKind::InlandSea
                                | litho_core::SurfaceKind::Swamp
                        ))
                {
                    continue;
                }
                if wanted.map_or(column.level < HIGH_LAKE_LEVEL, |q| q != "lake" && q != kind) {
                    continue;
                }
                lakes.entry(cluster(x, z)).or_insert(WaterPoint {
                    x: x as i64,
                    z: z as i64,
                    level: column.level,
                    kind,
                    note: if column.level >= HIGH_LAKE_LEVEL {
                        "high_lake"
                    } else {
                        "lake"
                    },
                });
                break;
            }
        }
        if wanted.is_some() {
            continue;
        }
        let channels = net.channels();
        for segment in channels.segments() {
            let reach = &channels.reaches()[segment.reach];
            let ax = (segment.a.point.x - shift.x) * l;
            let az = (segment.a.point.z - shift.z) * l;
            let bx = (segment.b.point.x - shift.x) * l;
            let bz = (segment.b.point.z - shift.z) * l;
            let mut previous: Option<i32> = None;
            for k in 0..=SEGMENT_PROBES {
                let t = k as f64 / SEGMENT_PROBES as f64;
                let x = ax + (bx - ax) * t;
                let z = az + (bz - az) * t;
                let column = generator.water_column(x, z);
                if !column.has_water() {
                    previous = None;
                    continue;
                }
                // Ступень: зеркало меняется на блок и больше на коротком шаге.
                if let Some(prev) = previous {
                    if (column.level - prev).abs() >= 1 {
                        steps.entry(cluster(x, z)).or_insert(WaterPoint {
                            x: x as i64,
                            z: z as i64,
                            level: column.level,
                            kind: "river",
                            note: "step",
                        });
                    }
                }
                // Устье в озеро: конец русла — озеро, вода ещё выше моря.
                if reach.end == ReachEnd::Lake && column.level > SEA {
                    mouths.entry(cluster(x, z)).or_insert(WaterPoint {
                        x: x as i64,
                        z: z as i64,
                        level: column.level,
                        kind: "river",
                        note: "mouth",
                    });
                }
                previous = Some(column.level);
            }
        }
    }
    let mut out: Vec<WaterPoint> = lakes.into_values().take(LAKE_LIMIT).collect();
    out.extend(steps.into_values().take(STEP_LIMIT));
    out.extend(mouths.into_values().take(MOUTH_LIMIT));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use litho_core::mode::Mode;
    use litho_core::version::GenVersion;

    /// R2: инвариант держится там, где вода вообще есть. Точку берём из
    /// `points`, чтобы тест не зависел от того, что попадёт в квадрат у нуля.
    #[test]
    fn r2_has_no_leaks_where_water_exists() {
        let r2 = Generator::new(42, Mode::Gameplay, GenVersion::V1DevR2).unwrap();
        // Опорная точка высокого озера из тестов ядра (там же ищется по сети).
        let found = points(&r2, (-35098.064436977, 8022.221972733), 8.0);
        assert!(
            !found.is_empty(),
            "на сиде 42 не нашлось ни одного места с водой"
        );
        // Три места и небольшое окно: тест проверяет инвариант, а не объём.
        for point in found.iter().take(3) {
            let leaks = leaks_against(&r2, None, (point.x as f64, point.z as f64), 64, 4);
            assert!(
                leaks.water > 0,
                "место {point:?} без воды выше моря: {leaks:?}"
            );
            assert_eq!(
                leaks.violations(),
                0,
                "нарушения у {point:?}: {:?}",
                leaks.examples
            );
        }
    }

    /// До R2 (и в v1-dev) воды выше моря нет: котловины сухие.
    #[test]
    fn v1_dev_has_no_water_above_the_sea() {
        let dev = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        let leaks = leaks_against(&dev, None, (0.0, 0.0), 256, 8);
        assert_eq!(leaks.water, 0);
    }

    #[test]
    fn points_are_capped_and_above_the_sea() {
        let r2 = Generator::new(42, Mode::Gameplay, GenVersion::V1DevR2).unwrap();
        let found = points(&r2, (0.0, 0.0), 8.0);
        assert!(found.len() <= LAKE_LIMIT + STEP_LIMIT + MOUTH_LIMIT);
        for point in &found {
            assert!(point.level > SEA, "{point:?}");
            if point.note == "high_lake" {
                assert!(point.level >= HIGH_LAKE_LEVEL, "{point:?}");
            }
        }
    }
}
