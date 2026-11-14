//! Сеть стока плиты (PLAN-REVISION 5.3, T110/T111).
//!
//! Здесь топография, годовой водный баланс и геометрия русел: узлы, выходы,
//! priority-flood, впадины, климат, расход Q и полилинии T112. Влияние на
//! C/E/W/PV — T115. Модуль не меняет v0 и не вызывается из `field`.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::channel::ChannelNetwork;
use crate::hash::unit;
use crate::lake::LakeNetwork;
use crate::math::{Vec2, smoothstep};
use crate::params::Params;
use crate::plates::{Plate, plate_query};
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
        }
    }
}

/// Вход топографического расчёта. `route_km` — высота стока, не игровая y.
/// Пока потенциал рельефа A (T120) не готов, генератор подаёт сюда `e_km`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrainageInput {
    pub route_km: f64,
    pub outlet: DrainageOutlet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiverNetError {
    UnsupportedVersion(GenVersion),
    InvalidParameters(&'static str),
    NoOwnedNode { plate_id: u64 },
    InvalidInput { cell: Cell },
    InvalidClimate { cell: Cell },
    BrokenInvariant { node: usize, reason: &'static str },
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
    pub filled_km: f64,
    pub downstream: Option<usize>,
    pub outlet: DrainageOutlet,
    pub depression: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiverNet {
    plate_id: u64,
    /// Шаг решётки нужен компактному запросу глубины впадины T120.
    step_l: f64,
    /// Рамка клеток узлов (min, max): точка вне рамки с запасом в две клетки
    /// не видит ни одного узла, и запрос глубины впадины отвечает нулём сразу.
    cell_bounds: Option<(Cell, Cell)>,
    nodes: Vec<DrainageNode>,
    depressions: Vec<Depression>,
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

/// Env-gated фаза-замер сборки сети (T79 вторая часть): печатает в stdout
/// строку `drainage-фазы: <имя> <мс>` только при `LITHO_DRAINAGE_TIMINGS=1`.
/// Математика не меняется; без переменной окружения — роспуск незаметен.
fn log_phase(enabled: bool, label: &str, started: std::time::Instant) {
    if enabled {
        println!(
            "drainage-фазы: {label} {:.3} мс",
            started.elapsed().as_secs_f64() * 1e3
        );
    }
}

impl RiverNet {
    pub fn build(
        plate: &Plate,
        seed: u64,
        params: &Params,
        input: &impl Fn(Vec2) -> DrainageInput,
        climate: &(impl Fn(Vec2) -> Option<RunoffClimate> + Sync),
    ) -> Result<Self, RiverNetError> {
        if !params.version.is_water_relief_dev() {
            return Err(RiverNetError::UnsupportedVersion(params.version));
        }
        let timings = std::env::var_os("LITHO_DRAINAGE_TIMINGS").is_some();
        let t_total = std::time::Instant::now();
        let mut t = std::time::Instant::now();
        validate_params(params)?;
        let p = &params.drainage;
        let anchor = find_anchor(plate, seed, params)?;
        log_phase(timings, "якорь", t);
        t = std::time::Instant::now();
        let (mut owned, ring) = collect_cells(anchor, plate.id, seed, params);
        owned.sort_unstable();
        let mut cells = owned.clone();
        cells.extend(ring);
        cells.sort_unstable();
        log_phase(timings, "клетки", t);
        let mut index = HashMap::with_capacity(cells.len());
        let mut nodes = Vec::with_capacity(cells.len());
        t = std::time::Instant::now();
        for (k, cell) in cells.into_iter().enumerate() {
            index.insert(cell, k);
            let point = jittered_point(cell, seed, params);
            let sample = input(point);
            if !sample.route_km.is_finite() {
                return Err(RiverNetError::InvalidInput { cell });
            }
            nodes.push(DrainageNode {
                cell,
                point,
                in_plate: owned.binary_search(&cell).is_ok(),
                route_km: sample.route_km,
                filled_km: sample.route_km,
                downstream: None,
                outlet: sample.outlet,
                depression: None,
            });
        }
        log_phase(timings, "узлы", t);
        t = std::time::Instant::now();
        let neighbors = grid_neighbors(&nodes, &index);
        log_phase(timings, "соседи", t);
        t = std::time::Instant::now();
        let depressions = route_nodes(
            &mut nodes,
            &neighbors,
            p.fill_epsilon_km,
            p.depression_min_depth_km,
            p.lake_area_max_fraction,
        );
        log_phase(timings, "flood", t);
        t = std::time::Instant::now();
        let points: Vec<_> = nodes.iter().map(|node| node.point).collect();
        let grid = ClimateGrid::build(&points, p.climate_step_l, climate)?;
        log_phase(timings, "климат", t);
        t = std::time::Instant::now();
        let runoff = Runoff::calculate(&nodes, &depressions, &grid, params)?;
        log_phase(timings, "сток", t);
        t = std::time::Instant::now();
        let channels = ChannelNetwork::build(plate.id, seed, &nodes, &runoff, params)?;
        log_phase(timings, "русла", t);
        t = std::time::Instant::now();
        let lakes = LakeNetwork::build(plate.id, &nodes, &depressions, &runoff, params)?;
        log_phase(timings, "озёра", t);
        let net = Self {
            plate_id: plate.id,
            step_l: p.step_l,
            cell_bounds: cell_bounds(&nodes),
            nodes,
            depressions,
            runoff,
            channels,
            lakes,
        };
        t = std::time::Instant::now();
        net.check_invariants(p.fill_epsilon_km)?;
        log_phase(timings, "инварианты", t);
        log_phase(timings, "итого", t_total);
        Ok(net)
    }

    pub const fn plate_id(&self) -> u64 {
        self.plate_id
    }

    pub fn nodes(&self) -> &[DrainageNode] {
        &self.nodes
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

    pub fn depressions(&self) -> &[Depression] {
        &self.depressions
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
        self.nodes.iter().filter(|node| node.in_plate).count()
    }

    pub fn outlet_count(&self) -> usize {
        self.nodes
            .iter()
            .filter(|node| node.outlet.is_outlet())
            .count()
    }

    /// Проверка для диагностики и тестов: заполнение не ниже рельефа, поток
    /// идёт строго вниз по заполненной высоте, выходы терминальны.
    pub fn check_invariants(&self, epsilon_km: f64) -> Result<(), RiverNetError> {
        for (k, node) in self.nodes.iter().enumerate() {
            let reason = if !node.route_km.is_finite() || !node.filled_km.is_finite() {
                Some("неконечная высота")
            } else if node.filled_km < node.route_km {
                Some("заполнение ниже рельефа")
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
fn collect_cells(
    anchor: Cell,
    plate_id: u64,
    seed: u64,
    params: &Params,
) -> (Vec<Cell>, Vec<Cell>) {
    let mut status = HashMap::<Cell, bool>::new();
    let mut queue = VecDeque::new();
    let mut owned = Vec::new();
    let mut ring = Vec::new();
    status.insert(anchor, true);
    queue.push_back(anchor);
    owned.push(anchor);
    while let Some(cell) = queue.pop_front() {
        for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let Some(next) = neighbor_cell(cell, di, dj) else {
                continue;
            };
            if status.contains_key(&next) {
                continue;
            }
            let point = jittered_point(next, seed, params);
            let is_owned = plate_query(point, seed, params).is_some_and(|s| s.a.id == plate_id);
            status.insert(next, is_owned);
            if is_owned {
                owned.push(next);
                queue.push_back(next);
            } else {
                ring.push(next);
            }
        }
    }
    (owned, ring)
}

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

/// Priority-flood с ε-подъёмом (Barnes et al., 2014). Все выходы стартуют в
/// очереди сразу; незамкнутые узлы получают родителя ниже по течению.
fn route_nodes(
    nodes: &mut [DrainageNode],
    neighbors: &[Vec<usize>],
    epsilon_km: f64,
    min_depth_km: f64,
    lake_area_max_fraction: f64,
) -> Vec<Depression> {
    let mut visited = vec![false; nodes.len()];
    let mut heap = BinaryHeap::new();
    for (k, node) in nodes.iter_mut().enumerate() {
        node.filled_km = node.route_km;
        node.downstream = None;
        node.depression = None;
        if node.outlet.is_outlet() {
            visited[k] = true;
            heap.push(QueueEntry {
                filled_km: node.route_km,
                node: k,
            });
        }
    }
    while let Some(entry) = heap.pop() {
        for &next in &neighbors[entry.node] {
            if visited[next] {
                continue;
            }
            visited[next] = true;
            let level = entry.filled_km + epsilon_km;
            nodes[next].filled_km = nodes[next].route_km.max(level);
            nodes[next].downstream = Some(entry.node);
            heap.push(QueueEntry {
                filled_km: nodes[next].filled_km,
                node: next,
            });
        }
    }

    let mut depressions = Vec::new();
    let closed: Vec<bool> = visited.iter().map(|v| !v).collect();
    add_components(nodes, neighbors, &closed, true, &mut depressions);
    let flooded: Vec<bool> = nodes
        .iter()
        .enumerate()
        .map(|(k, node)| visited[k] && node.filled_km - node.route_km > min_depth_km)
        .collect();
    add_components(nodes, neighbors, &flooded, false, &mut depressions);
    breach_over_budget(nodes, &mut depressions, lake_area_max_fraction);
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

type CachedNet = Result<Arc<RiverNet>, RiverNetError>;
type Slot = Arc<OnceLock<CachedNet>>;

#[derive(Debug)]
struct Shard {
    entries: HashMap<u64, (Slot, u64)>,
    capacity: usize,
}

/// Шардированный LRU слотов `OnceLock`. Под блокировкой только поиск/вставка
/// указателя; сама сеть считается вне блокировки. Разные плиты не ждут друг
/// друга. Вытеснение cold-слота допустимо: ответ остаётся чистой функцией.
#[derive(Debug)]
pub struct RiverNetCache {
    shards: Vec<Mutex<Shard>>,
    clock: AtomicU64,
}

impl RiverNetCache {
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
        build: impl FnOnce() -> Result<RiverNet, RiverNetError>,
    ) -> Result<Arc<RiverNet>, RiverNetError> {
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

impl Default for RiverNetCache {
    fn default() -> Self {
        Self::new(NET_CACHE_CAPACITY, NET_CACHE_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ClimateGrid, DepressionKind, DrainageNode, DrainageOutlet, RiverNet, RiverNetCache, Runoff,
        RunoffClimate, grid_neighbors, route_nodes,
    };
    use crate::channel::ChannelNetwork;
    use crate::lake::LakeNetwork;
    use crate::math::Vec2;
    use crate::version::GenVersion;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    fn tiny_net() -> RiverNet {
        let mut nodes = vec![
            node(0, 0, 1.0, DrainageOutlet::Sea),
            node(1, 0, 2.0, DrainageOutlet::None),
        ];
        let neighbors = prepare(&mut nodes);
        let depressions = route_nodes(&mut nodes, &neighbors, 1e-4, 0.002, 1.0);
        let params = crate::Params::defaults(GenVersion::V1Dev, crate::Mode::Gameplay);
        let points: Vec<_> = nodes.iter().map(|node| node.point).collect();
        let grid = ClimateGrid::build(&points, params.drainage.climate_step_l, |_| {
            Some(RunoffClimate {
                p_mm: 100.0,
                pet_mm: 0.0,
                t_min_c: 10.0,
            })
        })
        .unwrap();
        let runoff = Runoff::calculate(&nodes, &depressions, &grid, &params).unwrap();
        let channels = ChannelNetwork::build(7, 42, &nodes, &runoff, &params).unwrap();
        let lakes = LakeNetwork::build(7, &nodes, &depressions, &runoff, &params).unwrap();
        RiverNet {
            plate_id: 7,
            step_l: params.drainage.step_l,
            cell_bounds: super::cell_bounds(&nodes),
            nodes,
            depressions,
            runoff,
            channels,
            lakes,
        }
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
            let depressions = route_nodes(&mut nodes, &neighbors, 1e-4, 0.002, fraction);
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
        let depressions = route_nodes(&mut nodes, &neighbors, 1e-4, 0.002, 1.0);
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
        let depressions = route_nodes(&mut nodes, &neighbors, 1e-4, 0.002, 1.0);
        assert!(nodes.iter().all(|node| node.downstream.is_none()));
        assert!(nodes.iter().all(|node| node.outlet == DrainageOutlet::None));
        assert_eq!(depressions.len(), 1);
        assert_eq!(depressions[0].kind, DepressionKind::Closed);
        assert_eq!(depressions[0].node_count, 4);
    }

    #[test]
    fn depression_depth_query_is_compact_and_continuous() {
        let mut net = tiny_net();
        net.step_l = 1.0;
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
        let mut net = tiny_net();
        net.step_l = 1.0;
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
