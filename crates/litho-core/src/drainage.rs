//! Сеть стока плиты (PLAN-REVISION 5.3, T110/T111).
//!
//! Здесь топография, годовой водный баланс и геометрия русел: узлы, выходы,
//! priority-flood, впадины, климат, расход Q и полилинии T112. Влияние на
//! C/E/W/PV — T115. Модуль не меняет v0 и не вызывается из `field`.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::channel::ChannelNetwork;
use crate::hash::unit;
use crate::lake::LakeNetwork;
use crate::math::{Vec2, smoothstep};
use crate::params::Params;
use crate::plates::{Plate, PlateSample, plate_query};
use crate::runoff::{ClimateGrid, Runoff, RunoffClimate};
use crate::seed::{SALT_RIVERS_DRAINAGE, cell_hash, subseed};
use crate::version::GenVersion;

type Cell = (i64, i64);

/// Инфраструктурный LRU, не параметр мира: сеть крупнее coarse-тайла, поэтому
/// держим меньше плит. Вытеснение меняет только скорость, не ответ.
pub const NET_CACHE_CAPACITY: usize = 256;

/// Куда может уйти вода из узла. `None` — не выход.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrainageOutlet {
    #[default]
    None,
    Sea,
    Rift,
    Transform,
    /// Шаг 2 плана (T113): узел кольца, которому сеть передаёт воду.
    /// Продолжение русла и расход ведёт сеть плиты-владельца узла.
    Handoff,
}

impl DrainageOutlet {
    pub const fn is_outlet(self) -> bool {
        !matches!(self, Self::None)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Sea => "sea",
            Self::Rift => "rift",
            Self::Transform => "transform",
            Self::Handoff => "handoff",
        }
    }
}

/// Вход топографического расчёта. `route_km` — высота стока, не игровая y.
/// До W это `e_km`; W переводит высоту S0 с малой поправкой коры в км.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrainageInput {
    pub route_km: f64,
    /// Dominant morphoclimate zone for W depression policy.
    pub zone: usize,
    pub lake_signals: crate::water_profile::LakeSignals,
    pub outlet: DrainageOutlet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiverNetError {
    UnsupportedVersion(GenVersion),
    InvalidParameters(&'static str),
    NoOwnedNode {
        plate_id: u64,
    },
    InvalidInput {
        cell: Cell,
    },
    InvalidClimate {
        cell: Cell,
    },
    BrokenInvariant {
        node: usize,
        reason: &'static str,
    },
    /// Цикл стока: звено является своим предком вверх по течению. Уровень
    /// входа на такой ветви неопределён; вызовы сборки по такому звену
    /// завершаются этой ошибкой, а не переполнением стека.
    Cycle {
        node: usize,
    },
}

impl std::fmt::Display for RiverNetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion(v) => {
                write!(f, "сеть стока доступна только в v1-dev, не в {v:?}")
            }
            Self::InvalidParameters(reason) => write!(f, "неверные параметры стока: {reason}"),
            Self::NoOwnedNode { plate_id } => {
                write!(f, "у плиты {plate_id} не найден ни один узел в окне поиска")
            }
            Self::InvalidInput { cell } => {
                write!(f, "неконечная высота стока в узле {cell:?}")
            }
            Self::InvalidClimate { cell } => {
                write!(f, "некорректный или отсутствующий климат в узле {cell:?}")
            }
            Self::BrokenInvariant { node, reason } => {
                write!(f, "нарушен инвариант стока в узле {node}: {reason}")
            }
            Self::Cycle { node } => {
                write!(f, "цикл стока через узел {node}: звено — свой предок")
            }
        }
    }
}

impl std::error::Error for RiverNetError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DepressionKind {
    /// Заполненная впадина. `spill_km` — уровень priority-flood у дна,
    /// включая ε-наклон трассы; точное зеркало озера уточнит T145.
    Filled { spill_km: f64 },
    /// Топографически замкнутая область без найденного выхода. Это ещё не
    /// доказанное бессточное озеро: приток/испарение посчитает T111.
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Depression {
    pub kind: DepressionKind,
    pub bottom_node: usize,
    pub node_count: usize,
    pub plate_node_count: usize,
    pub max_depth_km: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrainageNode {
    pub cell: Cell,
    /// Макро-координаты в L. Джиттер общий для мира, поэтому соседние плиты
    /// видят один и тот же узел на общей границе.
    pub point: Vec2,
    pub in_plate: bool,
    pub route_km: f64,
    /// Уровень priority-flood. У узла-передачи (`DrainageOutlet::Handoff`) —
    /// уровень ухода воды в сети плиты-владельца, он не ниже `route_km`.
    pub filled_km: f64,
    pub downstream: Option<usize>,
    pub outlet: DrainageOutlet,
    pub depression: Option<usize>,
}

/// Меньше этого числа клеток волны на поток обход делить невыгодно.
const CELLS_PARALLEL_MIN_CHUNK: usize = 48;

/// Меньше этого числа узлов на поток высоты считать параллельно невыгодно.
const TOPO_PARALLEL_MIN_CHUNK: usize = 512;

/// Узел топографии плиты до связывания с соседями (шаг 2 плана, T113).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TopoNode {
    pub cell: Cell,
    pub point: Vec2,
    pub in_plate: bool,
    /// Плита-владелец узла: у кольца — сосед, `None` — плита не найдена.
    pub owner: Option<u64>,
    pub route_km: f64,
    /// Dominant morphoclimate zone for W depression policy.
    pub zone: usize,
    pub lake_signals: crate::water_profile::LakeSignals,
    pub outlet: DrainageOutlet,
    /// Уровень, с которого вода узла уходит через собственные выходы плиты
    /// (море, рифт, трансформ), не заходя в соседние плиты. `INFINITY` —
    /// такого пути нет. У узлов кольца не считается.
    pub escape_km: f64,
}

/// Первая фаза сети плиты: узлы, высоты, выходы и заливка только по своим
/// узлам. От соседних плит не зависит, поэтому соседи могут читать её без
/// рекурсии. `escape_km` — верхняя граница итогового уровня узла в связанной
/// сети: у той выходов не меньше.
#[derive(Clone, Debug, PartialEq)]
pub struct PlateTopo {
    plate: Plate,
    nodes: Vec<TopoNode>,
    neighbors: Vec<Plate>,
}

impl PlateTopo {
    pub fn build(
        plate: &Plate,
        seed: u64,
        params: &Params,
        input: &(impl Fn(Vec2, Option<&PlateSample>) -> DrainageInput + Sync),
        threads: usize,
    ) -> Result<Self, RiverNetError> {
        if !params.version.is_water_relief_dev() {
            return Err(RiverNetError::UnsupportedVersion(params.version));
        }
        let timings = timings_enabled();
        let mut t = std::time::Instant::now();
        validate_params(params)?;
        let p = &params.drainage;
        let anchor = find_anchor(plate, seed, params)?;
        let mut neighbors: Vec<Plate> = Vec::new();
        log_phase(timings, "якорь", t);
        t = std::time::Instant::now();
        // Запрос плиты в узле делается один раз: его результат нужен и для
        // владельца клетки, и для высоты (Д-093).
        let mut cells = collect_cells(anchor, plate.id, seed, params, threads);
        cells.sort_unstable_by_key(|(cell, _)| *cell);
        log_phase(timings, "клетки", t);
        t = std::time::Instant::now();
        // Высоты узлов независимы друг от друга: считаем их кусками в
        // нескольких потоках. Порядок результата — порядок клеток.
        let sample_chunk = |chunk: &[(Cell, Option<PlateSample>)]| -> Vec<(Vec2, DrainageInput)> {
            chunk
                .iter()
                .map(|(cell, sample)| {
                    let point = jittered_point(*cell, seed, params);
                    (point, input(point, sample.as_ref()))
                })
                .collect()
        };
        let threads = threads.clamp(1, (cells.len() / TOPO_PARALLEL_MIN_CHUNK).max(1));
        let inputs: Vec<(Vec2, DrainageInput)> = if threads >= 2 {
            let size = cells.len().div_ceil(threads);
            std::thread::scope(|scope| {
                let handles: Vec<_> = cells
                    .chunks(size)
                    .map(|chunk| scope.spawn(|| sample_chunk(chunk)))
                    .collect();
                handles
                    .into_iter()
                    .flat_map(|handle| handle.join().expect("поток высот узлов не паникует"))
                    .collect()
            })
        } else {
            sample_chunk(&cells)
        };
        let mut nodes = Vec::with_capacity(cells.len());
        for ((cell, sample), (point, input)) in cells.into_iter().zip(inputs) {
            if !input.route_km.is_finite() || input.zone >= crate::params::MORPHO_ZONES {
                return Err(RiverNetError::InvalidInput { cell });
            }
            if input.outlet == DrainageOutlet::Handoff {
                return Err(RiverNetError::InvalidInput { cell });
            }
            let owner = sample.map(|s| s.a);
            nodes.push(TopoNode {
                cell,
                point,
                in_plate: owner.is_some_and(|o| o.id == plate.id),
                owner: owner.map(|o| o.id),
                route_km: input.route_km,
                zone: input.zone,
                lake_signals: input.lake_signals,
                outlet: input.outlet,
                escape_km: f64::INFINITY,
            });
            // Сосед нужен только там, где узел кольца — суша без выхода:
            // через море, рифт и трансформ вода между плитами не идёт, и
            // топография такого соседа не запрашивается.
            if let Some(owner) = owner {
                if owner.id != plate.id
                    && !input.outlet.is_outlet()
                    && !neighbors.iter().any(|n: &Plate| n.id == owner.id)
                {
                    neighbors.push(owner);
                }
            }
        }
        neighbors.sort_unstable_by_key(|n| n.id);
        log_phase(timings, "узлы", t);
        t = std::time::Instant::now();
        // Заливка только по своим узлам и настоящим выходам: чужой узел без
        // выхода непроходим. Иначе путь через кольцо занизил бы уровень, а в
        // связанной сети узел кольца терминален.
        let passable: Vec<bool> = nodes
            .iter()
            .map(|node| node.in_plate || node.outlet.is_outlet())
            .collect();
        let cells: Vec<Cell> = nodes.iter().map(|node| node.cell).collect();
        let neighbor_lists = cell_neighbors(&cells);
        let route: Vec<f64> = nodes.iter().map(|node| node.route_km).collect();
        let seeds: Vec<(usize, f64)> = nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.outlet.is_outlet())
            .map(|(k, node)| (k, node.route_km))
            .collect();
        let flood = priority_flood(
            &route,
            &seeds,
            &passable,
            &neighbor_lists,
            p.fill_epsilon_km,
        );
        for (k, node) in nodes.iter_mut().enumerate() {
            if node.in_plate && flood.visited[k] {
                node.escape_km = flood.level[k];
            }
        }
        log_phase(timings, "заливка своя", t);
        Ok(Self {
            plate: *plate,
            nodes,
            neighbors,
        })
    }

    pub const fn plate_id(&self) -> u64 {
        self.plate.id
    }

    pub const fn plate(&self) -> &Plate {
        &self.plate
    }

    pub fn nodes(&self) -> &[TopoNode] {
        &self.nodes
    }

    /// Плиты-владельцы узлов кольца без выхода (суша), по возрастанию id.
    /// Только с ними возможен обмен водой.
    pub fn neighbors(&self) -> &[Plate] {
        &self.neighbors
    }

    pub(crate) fn node_at(&self, cell: Cell) -> Option<&TopoNode> {
        self.nodes
            .binary_search_by_key(&cell, |node| node.cell)
            .ok()
            .map(|k| &self.nodes[k])
    }

    /// Уровень ухода воды своего узла без выхода; `None` — узел чужой, сам
    /// является выходом или воде из него уйти некуда.
    pub fn escape_level_km(&self, cell: Cell) -> Option<f64> {
        let node = self.node_at(cell)?;
        (node.in_plate && !node.outlet.is_outlet() && node.escape_km.is_finite())
            .then_some(node.escape_km)
    }

    /// Соседи, чьи узлы суши в кольце касаются хотя бы одного узла `wanted`.
    /// Только они могут передать воду в эти узлы. По возрастанию id.
    pub fn neighbors_touching(&self, wanted: &dyn Fn(Cell) -> bool) -> Vec<Plate> {
        let mut out: Vec<Plate> = Vec::new();
        for node in &self.nodes {
            if node.in_plate || node.outlet.is_outlet() {
                continue;
            }
            let Some(owner) = node.owner else {
                continue;
            };
            if out.iter().any(|plate| plate.id == owner) {
                continue;
            }
            let touches = (-1..=1).any(|di| {
                (-1..=1).any(|dj| {
                    (di != 0 || dj != 0) && neighbor_cell(node.cell, di, dj).is_some_and(wanted)
                })
            });
            if touches {
                if let Some(plate) = self.neighbors.iter().find(|plate| plate.id == owner) {
                    out.push(*plate);
                }
            }
        }
        out.sort_unstable_by_key(|plate| plate.id);
        out
    }

    /// Необходимое условие передачи воды из этой плиты в узлы плиты `target`:
    /// есть пара соседних узлов, где свой уровень ухода выше уровня соседа.
    /// В связанной сети уровень своего узла не выше `escape_km`, поэтому
    /// «нет» здесь означает «передачи нет точно»; «да» — только «возможно».
    /// `wanted` ограничивает узлы `target`, которые интересуют вызывающего.
    pub fn may_hand_to(&self, target: &PlateTopo, wanted: &dyn Fn(Cell) -> bool) -> bool {
        for node in &self.nodes {
            if node.in_plate
                || node.owner != Some(target.plate.id)
                || node.outlet.is_outlet()
                || !wanted(node.cell)
            {
                continue;
            }
            let Some(level) = target.escape_level_km(node.cell) else {
                continue;
            };
            for di in -1..=1 {
                for dj in -1..=1 {
                    if di == 0 && dj == 0 {
                        continue;
                    }
                    let Some(next) = neighbor_cell(node.cell, di, dj) else {
                        continue;
                    };
                    if self.node_at(next).is_some_and(|own| {
                        own.in_plate && !own.outlet.is_outlet() && own.escape_km > level
                    }) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Вторая фаза: топография плиты, связанная с соседями. Узел кольца, из
/// которого вода соседа уходит на конечном уровне, становится выходом-
/// передачей с этим уровнем; остальные узлы кольца без выхода в сеть не
/// входят. Расхода здесь ещё нет.
///
/// Почему передачи не образуют цикла. Уровень передачи в узел `r` — это
/// `escape_km` узла у владельца, верхняя граница его итогового уровня. Вода
/// входит в `r` с уровня строго выше и дальше течёт строго вниз, поэтому
/// итоговый уровень узла у своего владельца убывает вдоль любого пути по
/// всем плитам.
#[derive(Clone, Debug)]
pub struct PlateRouting {
    plate: Plate,
    /// Шаг решётки нужен компактному запросу глубины впадины T120.
    step_l: f64,
    /// Рамка клеток узлов (min, max): точка вне рамки с запасом в две клетки
    /// не видит ни одного узла, и запрос глубины впадины отвечает нулём сразу.
    cell_bounds: Option<(Cell, Cell)>,
    nodes: Vec<DrainageNode>,
    depressions: Vec<Depression>,
    neighbors: Vec<Plate>,
}

impl PlateRouting {
    /// `ring_level(владелец, клетка)` — `PlateTopo::escape_level_km` соседа.
    pub fn build(
        topo: &PlateTopo,
        params: &Params,
        ring_level: &dyn Fn(u64, Cell) -> Option<f64>,
    ) -> Result<Self, RiverNetError> {
        if !params.version.is_water_relief_dev() {
            return Err(RiverNetError::UnsupportedVersion(params.version));
        }
        validate_params(params)?;
        let p = &params.drainage;
        let timings = timings_enabled();
        let t = std::time::Instant::now();
        let mut nodes = Vec::with_capacity(topo.nodes.len());
        let mut w_zones = params.version.is_water_update().then(Vec::new);
        for source in &topo.nodes {
            let (outlet, filled_km) = if source.in_plate || source.outlet.is_outlet() {
                (source.outlet, source.route_km)
            } else {
                let level = source
                    .owner
                    .and_then(|owner| ring_level(owner, source.cell))
                    .filter(|level| level.is_finite());
                match level {
                    Some(level) => (DrainageOutlet::Handoff, level.max(source.route_km)),
                    None => continue,
                }
            };
            if let Some(zones) = &mut w_zones {
                zones.push(source.zone);
            }
            nodes.push(DrainageNode {
                cell: source.cell,
                point: source.point,
                in_plate: source.in_plate,
                route_km: source.route_km,
                filled_km,
                downstream: None,
                outlet,
                depression: None,
            });
        }
        let cells: Vec<Cell> = nodes.iter().map(|node| node.cell).collect();
        let neighbor_lists = cell_neighbors(&cells);
        count_routing();
        count_route_nodes(nodes.len());
        let depressions = route_nodes(
            &mut nodes,
            &neighbor_lists,
            p.fill_epsilon_km,
            p.depression_min_depth_km,
            p.lake_area_max_fraction,
            w_zones.as_deref().map(|zones| (zones, params)),
        );
        log_phase(timings, "заливка связанная", t);
        let routing = Self {
            plate: topo.plate,
            step_l: p.step_l,
            cell_bounds: cell_bounds(&nodes),
            nodes,
            depressions,
            neighbors: topo.neighbors.clone(),
        };
        routing.check_invariants(p.fill_epsilon_km)?;
        Ok(routing)
    }

    pub const fn plate_id(&self) -> u64 {
        self.plate.id
    }

    pub const fn plate(&self) -> &Plate {
        &self.plate
    }

    pub fn nodes(&self) -> &[DrainageNode] {
        &self.nodes
    }

    pub fn depressions(&self) -> &[Depression] {
        &self.depressions
    }

    /// Плиты-владельцы узлов кольца, по возрастанию id.
    pub fn neighbors(&self) -> &[Plate] {
        &self.neighbors
    }

    /// Совпадают ли сети на узлах `mask`: тот же узел в той же клетке, тот же
    /// уровень, тот же сток вниз и та же впадина. Узлы вне маски не
    /// сравниваются. Нужна, чтобы подтвердить, что часть сети не зависит от
    /// неизвестных уровней третьих плит (Д-094).
    pub fn same_on(&self, mask: &[bool], other: &Self) -> bool {
        let down_cell = |net: &Self, k: usize| net.nodes[k].downstream.map(|d| net.nodes[d].cell);
        let basin = |net: &Self, k: usize| {
            net.nodes[k].depression.map(|d| {
                let depression = &net.depressions[d];
                (
                    depression.kind,
                    net.nodes[depression.bottom_node].cell,
                    depression.node_count,
                    depression.plate_node_count,
                )
            })
        };
        self.nodes.iter().enumerate().all(|(k, node)| {
            if !mask.get(k).copied().unwrap_or(false) {
                return true;
            }
            let Some(m) = other.node_index(node.cell) else {
                return false;
            };
            let twin = &other.nodes[m];
            twin.in_plate == node.in_plate
                && twin.outlet == node.outlet
                && twin.filled_km.to_bits() == node.filled_km.to_bits()
                && down_cell(self, k) == down_cell(other, m)
                && basin(self, k) == basin(other, m)
        })
    }

    pub fn node_index(&self, cell: Cell) -> Option<usize> {
        self.nodes
            .binary_search_by_key(&cell, |node| node.cell)
            .ok()
    }

    /// T120: непрерывная глубина впадины `filled_km - route_km` в точке.
    /// Узлы отсортированы по клетке; ядро с компактным носителем 2h не даёт
    /// скачка при смене набора соседних узлов и не требует новой решётки.
    pub fn depression_depth_km(&self, point: Vec2) -> f64 {
        if !point.x.is_finite() || !point.z.is_finite() || self.step_l <= 0.0 {
            return 0.0;
        }
        let h = self.step_l;
        let i = (point.x / h).floor() as i64;
        let j = (point.z / h).floor() as i64;
        // Окно запроса — клетки i±2, j±2. Если оно не пересекает рамку узлов,
        // ни один двоичный поиск ниже не найдёт узел и ответ равен нулю.
        match self.cell_bounds {
            Some((lo, hi)) => {
                if i.saturating_add(2) < lo.0
                    || i.saturating_sub(2) > hi.0
                    || j.saturating_add(2) < lo.1
                    || j.saturating_sub(2) > hi.1
                {
                    return 0.0;
                }
            }
            None => return 0.0,
        }
        let cutoff = 2.0 * h;
        let mut sum = 0.0;
        let mut weight = 0.0;
        for di in -2..=2 {
            for dj in -2..=2 {
                let Some(cell) = i.checked_add(di).zip(j.checked_add(dj)) else {
                    continue;
                };
                let Ok(k) = self.nodes.binary_search_by_key(&cell, |node| node.cell) else {
                    continue;
                };
                let node = &self.nodes[k];
                let d = point.sub(node.point).len();
                let w = 1.0 - smoothstep(0.0, cutoff, d);
                if w <= 0.0 {
                    continue;
                }
                let depth = (node.filled_km - node.route_km).max(0.0);
                sum += w * depth;
                weight += w;
            }
        }
        if weight > 0.0 { sum / weight } else { 0.0 }
    }

    /// Проверка для диагностики и тестов: заполнение не ниже рельефа, поток
    /// идёт строго вниз по заполненной высоте, выходы терминальны.
    pub fn check_invariants(&self, epsilon_km: f64) -> Result<(), RiverNetError> {
        for (k, node) in self.nodes.iter().enumerate() {
            let reason = if !node.route_km.is_finite() || !node.filled_km.is_finite() {
                Some("неконечная высота")
            } else if node.filled_km < node.route_km {
                Some("заполнение ниже рельефа")
            } else if node.outlet == DrainageOutlet::Handoff {
                if node.downstream.is_some() || node.in_plate || node.depression.is_some() {
                    Some("узел-передача не терминален или принадлежит плите")
                } else {
                    None
                }
            } else if node.outlet.is_outlet() {
                if node.downstream.is_some() || node.filled_km != node.route_km {
                    Some("выход не терминален или заполнен")
                } else {
                    None
                }
            } else if let Some(down) = node.downstream {
                if down >= self.nodes.len() {
                    Some("индекс ниже по течению вне сети")
                } else if node.filled_km < self.nodes[down].filled_km + epsilon_km {
                    Some("поток не идёт строго вниз")
                } else {
                    None
                }
            } else if node.depression.is_none() {
                Some("узел без выхода и без замкнутой впадины")
            } else {
                None
            };
            if let Some(reason) = reason {
                return Err(RiverNetError::BrokenInvariant { node: k, reason });
            }
            if let Some(d) = node.depression {
                if d >= self.depressions.len() {
                    return Err(RiverNetError::BrokenInvariant {
                        node: k,
                        reason: "индекс впадины вне сети",
                    });
                }
            }
        }
        Ok(())
    }
}

/// ВО3-скорость: производные сети соседа, построенные «только с этой плитой»
/// (Д-094). Обе — чистые функции топографии соседа и уровней ухода моей
/// плиты, поэтому кэшируются по паре id `(сосед, я)`: без кэша одна и та же
/// пара пересчитывалась на каждом шаге рекурсии притока.
///
/// `closed` — третьи плиты воду не принимают совсем; `open` — принимают на
/// самом низком возможном уровне. Если разбор Д-094 не сошёлся, вызывающий
/// строит настоящую сеть сам — кэш ей не нужен, она уже кэшируется отдельно.
#[derive(Debug)]
pub struct HandoffDerived {
    /// Сеть, где третьи плиты воду не принимают совсем.
    pub closed: Arc<PlateRouting>,
    /// Сеть, где третьи плиты принимают воду на высоте своего узла. `None` —
    /// у соседа нет третьих плит (все его соседи — я), и по Д-094 разбор
    /// сходится без неё: строить её значило бы платить впустую.
    pub open: Option<Arc<PlateRouting>>,
}

/// Сеть плиты целиком: связанная топография, расход с притоком от соседних
/// плит, русла и озёра.
#[derive(Clone, Debug, PartialEq)]
pub struct RiverNet {
    routing: Arc<PlateRouting>,
    runoff: Runoff,
    channels: ChannelNetwork,
    lakes: LakeNetwork,
}

/// Рамка клеток набора узлов; `None` для пустого набора.
fn cell_bounds(nodes: &[DrainageNode]) -> Option<(Cell, Cell)> {
    let first = nodes.first()?.cell;
    let mut lo = first;
    let mut hi = first;
    for node in nodes {
        lo.0 = lo.0.min(node.cell.0);
        lo.1 = lo.1.min(node.cell.1);
        hi.0 = hi.0.max(node.cell.0);
        hi.1 = hi.1.max(node.cell.1);
    }
    Some((lo, hi))
}

pub(crate) fn timings_enabled() -> bool {
    std::env::var_os("LITHO_DRAINAGE_TIMINGS").is_some()
}

/// Счётчики работы сборки сетей: сколько раз на самом деле построена каждая
/// фаза. Нужны, чтобы отделить «дорого на узел» от «дорого потому, что одна и
/// та же фаза строится много раз». На математику не влияют: только числа.
static STATS_ROUTINGS: AtomicU64 = AtomicU64::new(0);
static STATS_FLOODS: AtomicU64 = AtomicU64::new(0);
static STATS_ROUTE_NODES: AtomicU64 = AtomicU64::new(0);
static STATS_RUNOFFS: AtomicU64 = AtomicU64::new(0);
static STATS_CLIMATE_GRIDS: AtomicU64 = AtomicU64::new(0);
static STATS_CLIMATE_KEYS: AtomicU64 = AtomicU64::new(0);
static STATS_HANDOFFS: AtomicU64 = AtomicU64::new(0);
static STATS_MASKS: AtomicU64 = AtomicU64::new(0);

/// Снимок счётчиков сборки сетей (T-скорость, §6.2 ВО3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrainageStats {
    /// Сколько раз построен `PlateRouting` (связанная заливка плиты).
    pub routings: u64,
    /// Сколько раз выполнен `priority_flood`.
    pub floods: u64,
    /// Суммарное число узлов, прошедших через `route_nodes`.
    pub route_nodes: u64,
    /// Сколько раз посчитан полный годовой расход `Runoff::calculate_with`.
    pub runoffs: u64,
    /// Сколько раз построена решётка климата стока.
    pub climate_grids: u64,
    /// Сколько узлов решётки климата пробами (суммарно по всем сборкам).
    pub climate_keys: u64,
    /// Сколько раз вызван межплитовый приток `handoff_inflow`.
    pub handoffs: u64,
    /// Сколько раз посчитана маска водосбора `upstream_mask`.
    pub masks: u64,
}

impl DrainageStats {
    /// Доля повторных построек: сколько раз в среднем каждая фаза собиралась
    /// заново на один узел сети. Пусто — счётчики не снимались.
    pub fn summary(&self) -> String {
        format!(
            "routings={} floods={} runoffs={} climate_grids={} climate_keys={} handoffs={} masks={}",
            self.routings,
            self.floods,
            self.runoffs,
            self.climate_grids,
            self.climate_keys,
            self.handoffs,
            self.masks
        )
    }
}

/// Текущий снимок счётчиков.
pub fn drainage_stats() -> DrainageStats {
    DrainageStats {
        routings: STATS_ROUTINGS.load(AtomicOrdering::Relaxed),
        floods: STATS_FLOODS.load(AtomicOrdering::Relaxed),
        route_nodes: STATS_ROUTE_NODES.load(AtomicOrdering::Relaxed),
        runoffs: STATS_RUNOFFS.load(AtomicOrdering::Relaxed),
        climate_grids: STATS_CLIMATE_GRIDS.load(AtomicOrdering::Relaxed),
        climate_keys: STATS_CLIMATE_KEYS.load(AtomicOrdering::Relaxed),
        handoffs: STATS_HANDOFFS.load(AtomicOrdering::Relaxed),
        masks: STATS_MASKS.load(AtomicOrdering::Relaxed),
    }
}

/// Обнулить счётчики перед замером.
pub fn reset_drainage_stats() {
    for counter in [
        &STATS_ROUTINGS,
        &STATS_FLOODS,
        &STATS_ROUTE_NODES,
        &STATS_RUNOFFS,
        &STATS_CLIMATE_GRIDS,
        &STATS_CLIMATE_KEYS,
        &STATS_HANDOFFS,
        &STATS_MASKS,
    ] {
        counter.store(0, AtomicOrdering::Relaxed);
    }
}

pub(crate) fn count_routing() {
    STATS_ROUTINGS.fetch_add(1, AtomicOrdering::Relaxed);
}

pub(crate) fn count_route_nodes(nodes: usize) {
    STATS_ROUTE_NODES.fetch_add(nodes as u64, AtomicOrdering::Relaxed);
}

pub(crate) fn count_runoff() {
    STATS_RUNOFFS.fetch_add(1, AtomicOrdering::Relaxed);
}

pub(crate) fn count_climate_grid(keys: usize) {
    STATS_CLIMATE_GRIDS.fetch_add(1, AtomicOrdering::Relaxed);
    STATS_CLIMATE_KEYS.fetch_add(keys as u64, AtomicOrdering::Relaxed);
}

pub(crate) fn count_handoff() {
    STATS_HANDOFFS.fetch_add(1, AtomicOrdering::Relaxed);
}

pub(crate) fn count_mask() {
    STATS_MASKS.fetch_add(1, AtomicOrdering::Relaxed);
}

/// Env-gated фаза-замер сборки сети (T79 вторая часть): печатает в stdout
/// строку `drainage-фазы: <имя> <мс>` только при `LITHO_DRAINAGE_TIMINGS=1`.
/// Математика не меняется; без переменной окружения — роспуск незаметен.
pub(crate) fn log_phase(enabled: bool, label: &str, started: std::time::Instant) {
    if enabled {
        println!(
            "drainage-фазы: {label} {:.3} мс",
            started.elapsed().as_secs_f64() * 1e3
        );
    }
}

impl RiverNet {
    /// Третья фаза: расход, русла и озёра поверх связанной топографии.
    /// `inflow[k]` — вода, пришедшая в свой узел `k` из сетей соседних плит,
    /// мм·L²/год; пустой срез — притока нет.
    pub fn assemble(
        routing: Arc<PlateRouting>,
        seed: u64,
        params: &Params,
        climate: &(impl Fn(Vec2) -> Option<RunoffClimate> + Sync),
        inflow: &[f64],
    ) -> Result<Self, RiverNetError> {
        if !params.version.is_water_relief_dev() {
            return Err(RiverNetError::UnsupportedVersion(params.version));
        }
        validate_params(params)?;
        let p = &params.drainage;
        let timings = timings_enabled();
        let nodes = routing.nodes();
        let depressions = routing.depressions();
        let mut t = std::time::Instant::now();
        let points: Vec<_> = nodes
            .iter()
            .filter(|node| crate::runoff::needs_climate(node, params))
            .map(|node| node.point)
            .collect();
        let grid = ClimateGrid::build(&points, p.climate_step_l, climate)?;
        count_climate_grid(grid.len());
        log_phase(timings, "климат", t);
        t = std::time::Instant::now();
        let inflow = (!inflow.is_empty()).then_some(inflow);
        let runoff = Runoff::calculate_with(nodes, depressions, &grid, params, inflow, None)?;
        count_runoff();
        log_phase(timings, "сток", t);
        t = std::time::Instant::now();
        let channels = ChannelNetwork::build(routing.plate_id(), seed, nodes, &runoff, params)?;
        log_phase(timings, "русла", t);
        t = std::time::Instant::now();
        let lakes = LakeNetwork::build(routing.plate_id(), nodes, depressions, &runoff, params)?;
        log_phase(timings, "озёра", t);
        Ok(Self {
            routing,
            runoff,
            channels,
            lakes,
        })
    }

    pub fn plate_id(&self) -> u64 {
        self.routing.plate_id()
    }

    pub fn routing(&self) -> &Arc<PlateRouting> {
        &self.routing
    }

    pub fn nodes(&self) -> &[DrainageNode] {
        self.routing.nodes()
    }

    /// T120: глубина впадины в точке, см. [`PlateRouting::depression_depth_km`].
    pub fn depression_depth_km(&self, point: Vec2) -> f64 {
        self.routing.depression_depth_km(point)
    }

    pub fn depressions(&self) -> &[Depression] {
        self.routing.depressions()
    }

    pub const fn runoff(&self) -> &Runoff {
        &self.runoff
    }

    pub const fn channels(&self) -> &ChannelNetwork {
        &self.channels
    }

    pub const fn lakes(&self) -> &LakeNetwork {
        &self.lakes
    }

    pub fn plate_node_count(&self) -> usize {
        self.nodes().iter().filter(|node| node.in_plate).count()
    }

    pub fn outlet_count(&self) -> usize {
        self.nodes()
            .iter()
            .filter(|node| node.outlet.is_outlet())
            .count()
    }

    pub fn check_invariants(&self, epsilon_km: f64) -> Result<(), RiverNetError> {
        self.routing.check_invariants(epsilon_km)
    }
}

fn validate_params(params: &Params) -> Result<(), RiverNetError> {
    let p = &params.drainage;
    let reason = if !params.plates.cell_g_l.is_finite() || params.plates.cell_g_l <= 0.0 {
        Some("cell_g_l должен быть положительным")
    } else if !p.step_l.is_finite() || p.step_l <= 0.0 || params.plates.cell_g_l / p.step_l > 1e6 {
        Some("step_l должен быть положительным и не слишком малым относительно cell_g_l")
    } else if !p.jitter_fraction.is_finite() || !(0.0..0.5).contains(&p.jitter_fraction) {
        Some("jitter_fraction должен лежать в [0, 0.5)")
    } else if !p.fill_epsilon_km.is_finite() || p.fill_epsilon_km <= 0.0 {
        Some("fill_epsilon_km должен быть положительным")
    } else if !p.depression_min_depth_km.is_finite() || p.depression_min_depth_km < 0.0 {
        Some("depression_min_depth_km должен быть неотрицательным")
    } else if !p.lake_area_max_fraction.is_finite()
        || !(0.0..=1.0).contains(&p.lake_area_max_fraction)
    {
        Some("lake_area_max_fraction должен лежать в [0, 1]")
    } else if !p.outlet_min_weight.is_finite() || !(0.0..=1.0).contains(&p.outlet_min_weight) {
        Some("outlet_min_weight должен лежать в [0, 1]")
    } else if !(1..=8).contains(&p.anchor_search_radius_cells) {
        Some("anchor_search_radius_cells должен лежать в 1..=8")
    } else if !(0..=8).contains(&p.handoff_max_hops) {
        Some("handoff_max_hops должен лежать в 0..=8")
    } else if !p.climate_step_l.is_finite() || p.climate_step_l <= 0.0 {
        Some("climate_step_l должен быть положительным")
    } else if !p.et_factor.is_finite() || p.et_factor < 0.0 {
        Some("et_factor должен быть неотрицательным")
    } else {
        None
    };
    match reason {
        Some(reason) => Err(RiverNetError::InvalidParameters(reason)),
        None => Ok(()),
    }
}

fn neighbor_cell(cell: Cell, di: i64, dj: i64) -> Option<Cell> {
    Some((cell.0.checked_add(di)?, cell.1.checked_add(dj)?))
}

fn jittered_point(cell: Cell, seed: u64, params: &Params) -> Vec2 {
    let p = &params.drainage;
    let h = p.step_l;
    let salt = subseed(seed, SALT_RIVERS_DRAINAGE);
    let hx = cell_hash(salt, cell.0, cell.1, 0);
    let hz = cell_hash(salt, cell.0, cell.1, 1);
    let jx = (2.0 * unit(hx) - 1.0) * p.jitter_fraction * h;
    let jz = (2.0 * unit(hz) - 1.0) * p.jitter_fraction * h;
    Vec2::new(
        (cell.0 as f64 + 0.5) * h + jx,
        (cell.1 as f64 + 0.5) * h + jz,
    )
}

/// Ищем первый узел, принадлежащий плите. Окно ограничено параметром, но
/// отсутствие узла — явная ошибка, а не молчаливая пустая сеть.
fn find_anchor(plate: &Plate, seed: u64, params: &Params) -> Result<Cell, RiverNetError> {
    let p = &params.drainage;
    let base = (
        (plate.seed_pos.x / p.step_l).floor() as i64,
        (plate.seed_pos.z / p.step_l).floor() as i64,
    );
    let per_cell = (params.plates.cell_g_l / p.step_l).ceil() as i64;
    let max_radius = per_cell.saturating_mul(p.anchor_search_radius_cells).max(1);
    let mut radius = 2.min(max_radius);
    loop {
        let mut best: Option<(f64, Cell)> = None;
        for di in -radius..=radius {
            for dj in -radius..=radius {
                let Some(cell) = neighbor_cell(base, di, dj) else {
                    continue;
                };
                let point = jittered_point(cell, seed, params);
                if plate_query(point, seed, params).is_some_and(|s| s.a.id == plate.id) {
                    let d2 = point.dist2(plate.seed_pos);
                    if best.is_none_or(|(best_d2, _)| d2 < best_d2) {
                        best = Some((d2, cell));
                    }
                }
            }
        }
        if let Some((_, cell)) = best {
            return Ok(cell);
        }
        if radius >= max_radius {
            return Err(RiverNetError::NoOwnedNode { plate_id: plate.id });
        }
        radius = radius.saturating_mul(4).min(max_radius);
    }
}

/// Д-061: id плит, которым принадлежит хотя бы один узел решётки стока в
/// окне клеток `[lo, hi]` (включительно), по возрастанию. Признак владения
/// тот же, что в `collect_cells`: ближайшая плита в точке узла. Сеть плиты
/// состоит из её узлов и кольца в один узел, поэтому плита без узлов в окне,
/// расширенном на одну клетку, не имеет в самом окне ни одного узла сети.
/// Список зависит только от сида, параметров и окна.
#[cfg(test)]
pub(crate) fn owners_in_window(lo: Cell, hi: Cell, seed: u64, params: &Params) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    let mut last = None;
    for j in lo.1..=hi.1 {
        for i in lo.0..=hi.0 {
            let point = jittered_point((i, j), seed, params);
            let Some(sample) = plate_query(point, seed, params) else {
                continue;
            };
            let id = sample.a.id;
            if last != Some(id) {
                last = Some(id);
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
    }
    out.sort_unstable();
    out
}

/// T79: владельцы узлов решётки стока в блоке `side × side` клеток с углом
/// `(bx·side, bz·side)`, построчно. Признак владения тот же, что в
/// `owners_in_window`. Блок — чистая функция сида, параметров и своего номера.
pub(crate) fn owner_block(
    bx: i64,
    bz: i64,
    side: i64,
    seed: u64,
    params: &Params,
) -> Vec<Option<u64>> {
    let mut out = Vec::with_capacity((side * side) as usize);
    for j in 0..side {
        for i in 0..side {
            let cell = (
                bx.saturating_mul(side).saturating_add(i),
                bz.saturating_mul(side).saturating_add(j),
            );
            let point = jittered_point(cell, seed, params);
            out.push(plate_query(point, seed, params).map(|sample| sample.a.id));
        }
    }
    out
}

/// BFS только по узлам своей плиты; чужие соседи образуют кольцо в один узел.
/// Для узла кольца возвращается и плита-владелец: связанная сеть спрашивает
/// у неё уровень ухода воды.
fn collect_cells(
    anchor: Cell,
    plate_id: u64,
    seed: u64,
    params: &Params,
    threads: usize,
) -> Vec<(Cell, Option<PlateSample>)> {
    let query = |cell: Cell| plate_query(jittered_point(cell, seed, params), seed, params);
    let mut seen = HashSet::<Cell>::new();
    let mut cells = Vec::new();
    seen.insert(anchor);
    cells.push((anchor, query(anchor)));
    // Обход волнами: все новые соседи текущего фронта спрашиваются разом,
    // при большой волне — в нескольких потоках. Набор клеток тот же, что у
    // обхода по одной; порядок вызывающий задаёт сортировкой.
    let mut front = vec![anchor];
    let mut wave = Vec::new();
    while !front.is_empty() {
        wave.clear();
        for &cell in &front {
            for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                if let Some(next) = neighbor_cell(cell, di, dj) {
                    if seen.insert(next) {
                        wave.push(next);
                    }
                }
            }
        }
        let workers = threads.clamp(1, (wave.len() / CELLS_PARALLEL_MIN_CHUNK).max(1));
        let samples: Vec<Option<PlateSample>> = if workers >= 2 {
            let size = wave.len().div_ceil(workers);
            std::thread::scope(|scope| {
                let handles: Vec<_> = wave
                    .chunks(size)
                    .map(|chunk| {
                        scope.spawn(|| chunk.iter().map(|&cell| query(cell)).collect::<Vec<_>>())
                    })
                    .collect();
                handles
                    .into_iter()
                    .flat_map(|handle| handle.join().expect("поток обхода клеток не паникует"))
                    .collect()
            })
        } else {
            wave.iter().map(|&cell| query(cell)).collect()
        };
        front.clear();
        for (&cell, sample) in wave.iter().zip(samples) {
            if sample.is_some_and(|s| s.a.id == plate_id) {
                front.push(cell);
            }
            cells.push((cell, sample));
        }
    }
    cells
}

/// Восемь соседей по решётке для отсортированного списка клеток.
fn cell_neighbors(cells: &[Cell]) -> Vec<Vec<usize>> {
    let mut neighbors = vec![Vec::with_capacity(8); cells.len()];
    for (k, &cell) in cells.iter().enumerate() {
        for di in -1..=1 {
            for dj in -1..=1 {
                if di == 0 && dj == 0 {
                    continue;
                }
                if let Some(next) = neighbor_cell(cell, di, dj) {
                    if let Ok(m) = cells.binary_search(&next) {
                        neighbors[k].push(m);
                    }
                }
            }
        }
    }
    neighbors
}

#[cfg(test)]
fn grid_neighbors(nodes: &[DrainageNode], index: &HashMap<Cell, usize>) -> Vec<Vec<usize>> {
    let mut neighbors = vec![Vec::with_capacity(8); nodes.len()];
    for (k, node) in nodes.iter().enumerate() {
        for di in -1..=1 {
            for dj in -1..=1 {
                if di == 0 && dj == 0 {
                    continue;
                }
                if let Some(next) = neighbor_cell(node.cell, di, dj) {
                    if let Some(&m) = index.get(&next) {
                        neighbors[k].push(m);
                    }
                }
            }
        }
    }
    neighbors
}

#[derive(Clone, Copy, Debug)]
struct QueueEntry {
    filled_km: f64,
    node: usize,
}

impl PartialEq for QueueEntry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for QueueEntry {}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap — max-heap; переворачиваем высоту. Ничьи решаем меньшим
        // номером узла (узлы отсортированы по клетке), как требует план.
        other
            .filled_km
            .total_cmp(&self.filled_km)
            .then_with(|| other.node.cmp(&self.node))
    }
}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Итог priority-flood: уровень, родитель ниже по течению и признак обхода.
struct Flood {
    level: Vec<f64>,
    parent: Vec<Option<usize>>,
    visited: Vec<bool>,
}

/// Priority-flood с ε-подъёмом (Barnes et al., 2014). Все выходы `seeds`
/// стартуют в очереди сразу со своим уровнем; узлы с `passable = false` в
/// обход не входят.
fn priority_flood(
    route_km: &[f64],
    seeds: &[(usize, f64)],
    passable: &[bool],
    neighbors: &[Vec<usize>],
    epsilon_km: f64,
) -> Flood {
    let n = route_km.len();
    STATS_FLOODS.fetch_add(1, AtomicOrdering::Relaxed);
    let mut flood = Flood {
        level: route_km.to_vec(),
        parent: vec![None; n],
        visited: vec![false; n],
    };
    let mut heap = BinaryHeap::new();
    for &(k, level) in seeds {
        flood.visited[k] = true;
        flood.level[k] = level;
        heap.push(QueueEntry {
            filled_km: level,
            node: k,
        });
    }
    while let Some(entry) = heap.pop() {
        for &next in &neighbors[entry.node] {
            if flood.visited[next] || !passable[next] {
                continue;
            }
            flood.visited[next] = true;
            let level = entry.filled_km + epsilon_km;
            flood.level[next] = route_km[next].max(level);
            flood.parent[next] = Some(entry.node);
            heap.push(QueueEntry {
                filled_km: flood.level[next],
                node: next,
            });
        }
    }
    flood
}

/// Заливка сети и разбор впадин. Настоящий выход стартует со своей высоты,
/// узел-передача — с уровня, записанного в `filled_km` при сборке.
fn route_nodes(
    nodes: &mut [DrainageNode],
    neighbors: &[Vec<usize>],
    epsilon_km: f64,
    min_depth_km: f64,
    lake_area_max_fraction: f64,
    w_policy: Option<(&[usize], &Params)>,
) -> Vec<Depression> {
    let route: Vec<f64> = nodes.iter().map(|node| node.route_km).collect();
    let seeds: Vec<(usize, f64)> = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.outlet.is_outlet())
        .map(|(k, node)| {
            let level = if node.outlet == DrainageOutlet::Handoff {
                node.filled_km.max(node.route_km)
            } else {
                node.route_km
            };
            (k, level)
        })
        .collect();
    let passable = vec![true; nodes.len()];
    let flood = priority_flood(&route, &seeds, &passable, neighbors, epsilon_km);
    let visited = flood.visited;
    for (k, node) in nodes.iter_mut().enumerate() {
        node.filled_km = flood.level[k];
        node.downstream = flood.parent[k];
        node.depression = None;
    }

    let mut depressions = Vec::new();
    let closed: Vec<bool> = visited.iter().map(|v| !v).collect();
    add_components(nodes, neighbors, &closed, true, &mut depressions);
    // Выход во впадину не входит: у настоящего выхода заполнение равно
    // рельефу, а уровень узла-передачи — чужая вода, не своя яма.
    let flooded: Vec<bool> = nodes
        .iter()
        .enumerate()
        .map(|(k, node)| {
            visited[k] && !node.outlet.is_outlet() && node.filled_km - node.route_km > min_depth_km
        })
        .collect();
    add_components(nodes, neighbors, &flooded, false, &mut depressions);
    if let Some((zones, params)) = w_policy {
        breach_w_basins(nodes, &mut depressions, zones, params);
    } else {
        breach_over_budget(nodes, &mut depressions, lake_area_max_fraction);
    }
    depressions
}

/// К5 / Д-057: прорезка вместо заливки. Заполненные впадины плиты берутся от
/// самой глубокой, пока их узлы укладываются в долю `max_fraction` узлов суши
/// плиты. Остальные перестают быть впадинами: priority-flood уже дал им путь
/// к порогу, по нему идёт русло, а `filled_km` остаётся высотой прорези.
/// Замкнутые впадины без выхода прорезать некуда, они сохраняются.
fn breach_over_budget(
    nodes: &mut [DrainageNode],
    depressions: &mut Vec<Depression>,
    max_fraction: f64,
) {
    if max_fraction >= 1.0 || depressions.is_empty() {
        return;
    }
    let land = nodes
        .iter()
        .filter(|node| node.in_plate && !node.outlet.is_outlet())
        .count();
    let budget = (max_fraction.max(0.0) * land as f64).floor() as usize;
    let mut order: Vec<usize> = (0..depressions.len())
        .filter(|&d| {
            matches!(depressions[d].kind, DepressionKind::Filled { .. })
                && depressions[d].plate_node_count > 0
        })
        .collect();
    order.sort_by(|&a, &b| {
        depressions[b]
            .max_depth_km
            .total_cmp(&depressions[a].max_depth_km)
            .then_with(|| a.cmp(&b))
    });
    let mut keep = vec![true; depressions.len()];
    let mut used = 0_usize;
    for d in order {
        let size = depressions[d].plate_node_count;
        if used + size <= budget {
            used += size;
        } else {
            keep[d] = false;
        }
    }
    retain_basins(nodes, depressions, &keep);
}

/// §6.2: vote over the basin, not just its deepest node. Breach the
/// lowest weighted depths first until the *plate* area fits the W cap.
fn breach_w_basins(
    nodes: &mut [DrainageNode],
    depressions: &mut Vec<Depression>,
    zones: &[usize],
    params: &Params,
) {
    let mut votes = vec![[0usize; crate::params::MORPHO_ZONES]; depressions.len()];
    let mut plate_area = 0;
    for (node, &zone) in nodes.iter().zip(zones) {
        if node.in_plate {
            plate_area += 1;
            if let Some(d) = node.depression {
                votes[d][zone] += 1;
            }
        }
    }
    let mut keep = vec![true; depressions.len()];
    let mut ranked = Vec::new();
    let mut used = 0;
    for (d, basin) in depressions.iter().enumerate() {
        if !matches!(basin.kind, DepressionKind::Filled { .. }) {
            continue;
        }
        let zone = (0..votes[d].len())
            .max_by_key(|&z| (votes[d][z], std::cmp::Reverse(z)))
            .unwrap();
        let depth = basin.max_depth_km * params.height.water_km_to_blocks;
        if depth <= params.water.depression_min_depth_blocks[zone] {
            keep[d] = false;
        } else {
            let weight = [3., 3., 1.5, 0.5, 0., 1., 1., 1.][zone];
            ranked.push((depth * weight, d));
            used += basin.plate_node_count;
        }
    }
    let budget = (plate_area as f64 * params.water.lake_area_max_fraction).floor() as usize;
    ranked.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    for (_, d) in ranked {
        if used <= budget {
            break;
        }
        used -= depressions[d].plate_node_count;
        keep[d] = false;
    }
    retain_basins(nodes, depressions, &keep);
}

fn retain_basins(nodes: &mut [DrainageNode], depressions: &mut Vec<Depression>, keep: &[bool]) {
    if keep.iter().all(|k| *k) {
        return;
    }
    let mut remap = vec![None; depressions.len()];
    let mut next = 0_usize;
    for (d, kept) in keep.iter().enumerate() {
        if *kept {
            remap[d] = Some(next);
            next += 1;
        }
    }
    for node in nodes.iter_mut() {
        node.depression = node.depression.and_then(|d| remap[d]);
    }
    let mut d = 0_usize;
    depressions.retain(|_| {
        let kept = keep[d];
        d += 1;
        kept
    });
}

fn add_components(
    nodes: &mut [DrainageNode],
    neighbors: &[Vec<usize>],
    mask: &[bool],
    closed: bool,
    depressions: &mut Vec<Depression>,
) {
    for start in 0..nodes.len() {
        if !mask[start] || nodes[start].depression.is_some() {
            continue;
        }
        let id = depressions.len();
        let mut stack = vec![start];
        nodes[start].depression = Some(id);
        let mut members = Vec::new();
        while let Some(k) = stack.pop() {
            members.push(k);
            for &next in &neighbors[k] {
                if mask[next] && nodes[next].depression.is_none() {
                    nodes[next].depression = Some(id);
                    stack.push(next);
                }
            }
        }
        let bottom = *members
            .iter()
            .min_by(|&&a, &&b| {
                nodes[a]
                    .route_km
                    .total_cmp(&nodes[b].route_km)
                    .then_with(|| a.cmp(&b))
            })
            .expect("компонента непуста");
        let max_depth_km = members
            .iter()
            .map(|&k| nodes[k].filled_km - nodes[k].route_km)
            .fold(0.0, f64::max);
        depressions.push(Depression {
            kind: if closed {
                DepressionKind::Closed
            } else {
                DepressionKind::Filled {
                    spill_km: nodes[bottom].filled_km,
                }
            },
            bottom_node: bottom,
            node_count: members.len(),
            plate_node_count: members.iter().filter(|&&k| nodes[k].in_plate).count(),
            max_depth_km,
        });
    }
}

type Slot<T> = Arc<OnceLock<Result<Arc<T>, RiverNetError>>>;

#[derive(Debug)]
struct Shard<T> {
    entries: HashMap<u64, (Slot<T>, u64)>,
    capacity: usize,
}

/// Шардированный LRU слотов `OnceLock` по id плиты. Под блокировкой только
/// поиск/вставка указателя; само значение считается вне блокировки. Разные
/// плиты не ждут друг друга. Вытеснение cold-слота допустимо: ответ остаётся
/// чистой функцией.
#[derive(Debug)]
pub struct PlateCache<T> {
    shards: Vec<Mutex<Shard<T>>>,
    clock: AtomicU64,
}

/// Кэш готовых сетей плит.
pub type RiverNetCache = PlateCache<RiverNet>;

impl<T> PlateCache<T> {
    pub fn new(shards: usize, capacity: usize) -> Self {
        // Не даём числу шардов съесть вместимость: при 64/64 каждый шард имел
        // бы один слот и случайные коллизии id вытесняли бы горячие плиты.
        // Минимум четыре слота на шард, когда общая вместимость позволяет.
        let shards = shards.max(1).min((capacity / 4).max(1));
        let per_shard = (capacity / shards).max(1);
        Self {
            shards: (0..shards)
                .map(|_| {
                    Mutex::new(Shard {
                        entries: HashMap::new(),
                        capacity: per_shard,
                    })
                })
                .collect(),
            clock: AtomicU64::new(0),
        }
    }

    pub fn get_or_build(
        &self,
        plate_id: u64,
        build: impl FnOnce() -> Result<T, RiverNetError>,
    ) -> Result<Arc<T>, RiverNetError> {
        let shard = &self.shards[(plate_id as usize) % self.shards.len()];
        let slot = {
            let mut shard = shard.lock().expect("кэш сети стока не отравлен");
            if let Some((slot, stamp)) = shard.entries.get_mut(&plate_id) {
                *stamp = self.clock.fetch_add(1, AtomicOrdering::Relaxed);
                Arc::clone(slot)
            } else {
                if shard.entries.len() >= shard.capacity {
                    if let Some(oldest) = shard
                        .entries
                        .iter()
                        .min_by_key(|(_, (_, stamp))| *stamp)
                        .map(|(&key, _)| key)
                    {
                        shard.entries.remove(&oldest);
                    }
                }
                let slot = Arc::new(OnceLock::new());
                let stamp = self.clock.fetch_add(1, AtomicOrdering::Relaxed);
                shard.entries.insert(plate_id, (Arc::clone(&slot), stamp));
                slot
            }
        };
        slot.get_or_init(|| build().map(Arc::new)).clone()
    }

    /// Есть ли уже посчитанное значение; LRU не трогает.
    pub fn is_ready(&self, plate_id: u64) -> bool {
        let shard = &self.shards[(plate_id as usize) % self.shards.len()];
        let shard = shard.lock().expect("кэш сети стока не отравлен");
        shard
            .entries
            .get(&plate_id)
            .is_some_and(|(slot, _)| slot.get().is_some())
    }

    pub fn len(&self) -> usize {
        self.shards
            .iter()
            .map(|shard| {
                shard
                    .lock()
                    .expect("кэш сети стока не отравлен")
                    .entries
                    .len()
            })
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T> Default for PlateCache<T> {
    fn default() -> Self {
        Self::new(NET_CACHE_CAPACITY, NET_CACHE_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DepressionKind, DrainageNode, DrainageOutlet, Plate, PlateRouting, RiverNet, RiverNetCache,
        RunoffClimate, grid_neighbors, route_nodes,
    };
    use crate::math::Vec2;
    use crate::version::GenVersion;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Счётчики работы — только диагностика: обнуление и накопление не
    /// трогают математику. Параллельные тесты могут делить процесс, поэтому
    /// проверяем монотонность прибавки, а не абсолютное значение.
    #[test]
    fn work_counters_accumulate() {
        let before = super::drainage_stats();
        super::count_routing();
        super::count_runoff();
        super::count_handoff();
        super::count_mask();
        super::count_climate_grid(7);
        super::count_route_nodes(11);
        let after = super::drainage_stats();
        assert!(after.routings > before.routings);
        assert!(after.runoffs > before.runoffs);
        assert!(after.handoffs > before.handoffs);
        assert!(after.masks > before.masks);
        assert!(after.climate_grids > before.climate_grids);
        assert!(after.climate_keys >= before.climate_keys + 7);
        assert!(after.route_nodes >= before.route_nodes + 11);
        // `priority_flood` считается в своей точке, не здесь.
        assert!(after.summary().contains("routings="));
    }

    fn node(i: i64, j: i64, route_km: f64, outlet: DrainageOutlet) -> DrainageNode {
        DrainageNode {
            cell: (i, j),
            point: Vec2::new(i as f64, j as f64),
            in_plate: true,
            route_km,
            filled_km: route_km,
            downstream: None,
            outlet,
            depression: None,
        }
    }

    fn prepare(nodes: &mut [DrainageNode]) -> Vec<Vec<usize>> {
        let index: HashMap<_, _> = nodes
            .iter()
            .enumerate()
            .map(|(k, node)| (node.cell, k))
            .collect();
        grid_neighbors(nodes, &index)
    }

    fn test_plate(id: u64) -> Plate {
        Plate {
            id,
            cell_i: 0,
            cell_j: 0,
            index: 0,
            seed_pos: Vec2::new(0.0, 0.0),
            center: Vec2::new(0.0, 0.0),
            has_continent: true,
            radius_l: 1.0,
            aspect: 1.0,
            axis: Vec2::new(1.0, 0.0),
            velocity: Vec2::new(0.0, 0.0),
            omega: 0.0,
            age: 0.0,
        }
    }

    fn tiny_routing() -> PlateRouting {
        let mut nodes = vec![
            node(0, 0, 1.0, DrainageOutlet::Sea),
            node(1, 0, 2.0, DrainageOutlet::None),
        ];
        let neighbors = prepare(&mut nodes);
        let depressions = route_nodes(&mut nodes, &neighbors, 1e-4, 0.002, 1.0, None);
        PlateRouting {
            plate: test_plate(7),
            step_l: 1.0,
            cell_bounds: super::cell_bounds(&nodes),
            nodes,
            depressions,
            neighbors: Vec::new(),
        }
    }

    fn tiny_net() -> RiverNet {
        let params = crate::Params::defaults(GenVersion::V1Dev, crate::Mode::Gameplay);
        RiverNet::assemble(
            Arc::new(tiny_routing()),
            42,
            &params,
            &|_| {
                Some(RunoffClimate {
                    p_mm: 100.0,
                    pet_mm: 0.0,
                    t_min_c: 10.0,
                })
            },
            &[],
        )
        .unwrap()
    }

    #[test]
    fn over_budget_pits_are_breached_deepest_first() {
        // Д-057: две ямы на 7×7; бюджет в один узел оставляет глубокую,
        // мелкая прорезается и остаётся проточной.
        let build = |fraction: f64| {
            let mut nodes = Vec::new();
            for i in 0..7 {
                for j in 0..7 {
                    let height = if i == 0 && j == 3 {
                        0.0
                    } else if i == 2 && j == 2 {
                        0.2
                    } else if i == 4 && j == 4 {
                        0.6
                    } else {
                        1.0
                    };
                    let outlet = if i == 0 && j == 3 {
                        DrainageOutlet::Sea
                    } else {
                        DrainageOutlet::None
                    };
                    nodes.push(node(i, j, height, outlet));
                }
            }
            let neighbors = prepare(&mut nodes);
            let depressions = route_nodes(&mut nodes, &neighbors, 1e-4, 0.002, fraction, None);
            (nodes, depressions)
        };
        let deep = 2 * 7 + 2;
        let shallow = 4 * 7 + 4;
        let (nodes, depressions) = build(1.0);
        assert_eq!(depressions.len(), 2);
        assert!(nodes[deep].depression.is_some() && nodes[shallow].depression.is_some());

        let (nodes, depressions) = build(0.03);
        assert_eq!(depressions.len(), 1);
        assert_eq!(nodes[deep].depression, Some(0));
        assert_eq!(depressions[0].bottom_node, deep);
        assert_eq!(nodes[shallow].depression, None);
        assert!(nodes[shallow].downstream.is_some());
        assert!(nodes[shallow].filled_km > nodes[shallow].route_km);

        let (nodes, depressions) = build(0.0);
        assert!(depressions.is_empty());
        assert!(nodes.iter().all(|node| node.depression.is_none()));
        assert!(
            nodes
                .iter()
                .all(|node| node.outlet.is_outlet() || node.downstream.is_some())
        );
    }

    #[test]
    fn w_majority_zone_controls_depth_and_budget_priority() {
        let mut p = crate::Params::defaults(GenVersion::V1DevW, crate::Mode::Gameplay);
        // Two pits, one 3-cell polar depression (deepest node arid), one
        // deeper semiarid pit. Budget fits only the polar one.
        let mut nodes = Vec::new();
        let mut zones = Vec::new();
        for i in 0..10 {
            for j in 0..10 {
                let (depth, zone) = match (i, j) {
                    (3, 3) => (4., 4),
                    (2, 3) | (3, 2) => (3., 0),
                    (7, 7) => (8., 3),
                    _ => (0., 2),
                };
                zones.push(zone);
                nodes.push(node(
                    i,
                    j,
                    1.0 - depth / p.height.water_km_to_blocks,
                    if (i, j) == (0, 0) {
                        DrainageOutlet::Sea
                    } else {
                        DrainageOutlet::None
                    },
                ));
            }
        }
        p.water.lake_area_max_fraction = 0.03;
        let neighbors = prepare(&mut nodes);
        let basins = route_nodes(&mut nodes, &neighbors, 1e-6, 0.002, 1.0, Some((&zones, &p)));
        assert_eq!(basins.len(), 1);
        assert_eq!(basins[0].plate_node_count, 3);
        assert!(nodes[77].depression.is_none());
        zones.fill(4);
        assert!(
            route_nodes(&mut nodes, &neighbors, 1e-6, 0.002, 1.0, Some((&zones, &p))).is_empty()
        );
    }

    #[test]
    fn priority_flood_fills_pit_and_routes_to_sea() {
        let mut nodes = Vec::new();
        for i in 0..5 {
            for j in 0..5 {
                let height = if (i == 2 && j == 2) || (i == 0 && j == 2) {
                    0.0
                } else {
                    1.0
                };
                let outlet = if i == 0 && j == 2 {
                    DrainageOutlet::Sea
                } else {
                    DrainageOutlet::None
                };
                nodes.push(node(i, j, height, outlet));
            }
        }
        let neighbors = prepare(&mut nodes);
        let depressions = route_nodes(&mut nodes, &neighbors, 1e-4, 0.002, 1.0, None);
        let pit = 2 * 5 + 2;
        assert!(nodes[pit].filled_km > 1.0);
        assert!(nodes[pit].downstream.is_some());
        assert!(nodes[pit].depression.is_some());
        let d = &depressions[nodes[pit].depression.unwrap()];
        assert!(matches!(d.kind, DepressionKind::Filled { spill_km } if spill_km > 1.0));
        let mut current = pit;
        let mut steps = 0;
        while let Some(down) = nodes[current].downstream {
            assert!(nodes[current].filled_km >= nodes[down].filled_km + 1e-4);
            current = down;
            steps += 1;
            assert!(steps <= nodes.len());
        }
        assert_eq!(nodes[current].outlet, DrainageOutlet::Sea);
    }

    #[test]
    fn closed_basin_without_outlet_is_not_invented_sea() {
        let mut nodes = vec![
            node(0, 0, 2.0, DrainageOutlet::None),
            node(1, 0, 1.0, DrainageOutlet::None),
            node(0, 1, 2.0, DrainageOutlet::None),
            node(1, 1, 2.0, DrainageOutlet::None),
        ];
        let neighbors = prepare(&mut nodes);
        let depressions = route_nodes(&mut nodes, &neighbors, 1e-4, 0.002, 1.0, None);
        assert!(nodes.iter().all(|node| node.downstream.is_none()));
        assert!(nodes.iter().all(|node| node.outlet == DrainageOutlet::None));
        assert_eq!(depressions.len(), 1);
        assert_eq!(depressions[0].kind, DepressionKind::Closed);
        assert_eq!(depressions[0].node_count, 4);
    }

    #[test]
    fn depression_depth_query_is_compact_and_continuous() {
        let mut net = tiny_routing();
        net.nodes[1].filled_km = net.nodes[1].route_km + 0.5;
        let point = Vec2::new(1.0, 0.0);
        let depth = net.depression_depth_km(point);
        assert!(depth > 0.0 && depth <= 0.5);
        assert_eq!(net.depression_depth_km(Vec2::new(10.0, 10.0)), 0.0);
        let shifted = net.depression_depth_km(point.add(Vec2::new(1e-6, 0.0)));
        assert!((depth - shifted).abs() < 1e-5);
    }

    #[test]
    fn depression_depth_bounds_shortcut_does_not_change_the_answer() {
        let mut net = tiny_routing();
        net.nodes[1].filled_km = net.nodes[1].route_km + 0.5;
        assert_eq!(net.cell_bounds, Some(((0, 0), (1, 0))));
        // Та же сеть с рамкой «весь мир»: ранний выход не срабатывает никогда.
        let mut plain = net.clone();
        plain.cell_bounds = Some(((i64::MIN, i64::MIN), (i64::MAX, i64::MAX)));
        let mut positive = 0;
        let mut outside = 0;
        for a in -40..=40 {
            for b in -40..=40 {
                let point = Vec2::new(a as f64 * 0.173, b as f64 * 0.173);
                let fast = net.depression_depth_km(point);
                assert_eq!(fast.to_bits(), plain.depression_depth_km(point).to_bits());
                positive += usize::from(fast > 0.0);
                outside += usize::from(point.x < -2.0 || point.x > 4.0);
            }
        }
        assert!(positive > 100 && outside > 1000);
        let mut empty = net.clone();
        empty.nodes.clear();
        empty.cell_bounds = super::cell_bounds(&empty.nodes);
        assert_eq!(empty.cell_bounds, None);
        assert_eq!(empty.depression_depth_km(Vec2::new(1.0, 0.0)), 0.0);
    }

    #[test]
    fn cache_builds_once_and_eviction_is_deterministic() {
        let cache = RiverNetCache::new(1, 1);
        let builds = AtomicUsize::new(0);
        let first = cache
            .get_or_build(7, || {
                builds.fetch_add(1, Ordering::SeqCst);
                Ok(tiny_net())
            })
            .unwrap();
        let second = cache
            .get_or_build(7, || {
                builds.fetch_add(1, Ordering::SeqCst);
                Ok(tiny_net())
            })
            .unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        let _other = cache
            .get_or_build(8, || {
                builds.fetch_add(1, Ordering::SeqCst);
                Ok(tiny_net())
            })
            .unwrap();
        let rebuilt = cache
            .get_or_build(7, || {
                builds.fetch_add(1, Ordering::SeqCst);
                Ok(tiny_net())
            })
            .unwrap();
        assert_eq!(first, rebuilt);
        assert_eq!(builds.load(Ordering::SeqCst), 3);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn many_shards_do_not_reduce_each_shard_to_one_slot() {
        let cache = RiverNetCache::new(64, 64);
        let mut first = None;
        for k in 0..4_u64 {
            let net = cache.get_or_build(k * 64, || Ok(tiny_net())).unwrap();
            if k == 0 {
                first = Some(net);
            }
        }
        assert_eq!(cache.len(), 4);
        let again = cache.get_or_build(0, || Ok(tiny_net())).unwrap();
        assert!(Arc::ptr_eq(&first.unwrap(), &again));
    }
}
