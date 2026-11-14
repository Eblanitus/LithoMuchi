//! Климатический сток и расход поверх топографии T110 (PLAN-REVISION 5.3, T111).
//!
//! Единицы расхода — `мм·L²/год`: P и PET приходят в мм/год, площадь узла
//! равна `h²` в L². Перевод в м³/с и подбор `Q_min` относятся к T112/T160.
//! Кольцо чужой плиты передаёт воду, но не добавляет свой локальный сток.

use std::cmp::Ordering;
use std::collections::{BTreeSet, BinaryHeap, HashMap};

use crate::drainage::{Depression, DrainageNode, DrainageOutlet, RiverNetError};
use crate::math::{Vec2, lerp};
use crate::params::Params;

type Cell = (i64, i64);

/// P и PET в мм/год, неотрицательны и конечны. T114 добавляет температуру
/// самого холодного месяца для замёрзших озёр; она может быть отрицательной.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RunoffClimate {
    pub p_mm: f64,
    pub pet_mm: f64,
    pub t_min_c: f64,
}

/// Разреженная глобальная решётка климата с шагом T111. Считаются только
/// углы, нужные узлам сети; порядок вычисления фиксирован сортировкой ключей.
#[derive(Clone, Debug, PartialEq)]
pub struct ClimateGrid {
    step_l: f64,
    cells: HashMap<Cell, RunoffClimate>,
}

/// T79, вторая часть: ниже этого числа узлов климатической решётки потоки
/// дороже самой выборки. Ограничение сверху не даёт песочнице, которая уже
/// рисует карты в несколько потоков, размножать потоки на каждый чанк.
const CLIMATE_PARALLEL_MIN_KEYS: usize = 64;
const CLIMATE_PARALLEL_MAX_THREADS: usize = 4;

impl ClimateGrid {
    pub fn build<'a, F>(
        points: impl IntoIterator<Item = &'a Vec2>,
        step_l: f64,
        sample: F,
    ) -> Result<Self, RiverNetError>
    where
        F: Fn(Vec2) -> Option<RunoffClimate> + Sync,
    {
        if !step_l.is_finite() || step_l <= 0.0 {
            return Err(RiverNetError::InvalidParameters(
                "climate_step_l должен быть положительным",
            ));
        }
        let mut keys = BTreeSet::new();
        for point in points {
            if !point.x.is_finite() || !point.z.is_finite() {
                return Err(RiverNetError::InvalidParameters(
                    "узел стока имеет неконечные координаты",
                ));
            }
            let i = (point.x / step_l).floor() as i64;
            let j = (point.z / step_l).floor() as i64;
            for di in 0..=1 {
                for dj in 0..=1 {
                    let Some(key) = checked_cell(i, j, di, dj) else {
                        return Err(RiverNetError::InvalidParameters(
                            "решётка климата вышла за диапазон i64",
                        ));
                    };
                    keys.insert(key);
                }
            }
        }
        // BTreeSet задаёт порядок ключей; значения собираются в том же порядке,
        // поэтому параллельная выборка побитово равна последовательной.
        let keys: Vec<Cell> = keys.into_iter().collect();
        let values = sample_climate_keys(&keys, step_l, &sample)?;
        let mut cells = HashMap::with_capacity(keys.len());
        for (key, value) in keys.into_iter().zip(values) {
            cells.insert(key, value);
        }
        Ok(Self { step_l, cells })
    }

    pub fn sample(&self, point: Vec2) -> Result<RunoffClimate, RiverNetError> {
        let x = point.x / self.step_l;
        let z = point.z / self.step_l;
        let i = x.floor() as i64;
        let j = z.floor() as i64;
        let fx = x - i as f64;
        let fz = z - j as f64;
        let get = |di: i64, dj: i64| {
            let key =
                checked_cell(i, j, di, dj).ok_or(RiverNetError::InvalidClimate { cell: (i, j) })?;
            self.cells
                .get(&key)
                .copied()
                .ok_or(RiverNetError::InvalidClimate { cell: key })
        };
        let c00 = get(0, 0)?;
        let c10 = get(1, 0)?;
        let c01 = get(0, 1)?;
        let c11 = get(1, 1)?;
        let p_mm = lerp(
            lerp(c00.p_mm, c10.p_mm, fx),
            lerp(c01.p_mm, c11.p_mm, fx),
            fz,
        );
        let pet_mm = lerp(
            lerp(c00.pet_mm, c10.pet_mm, fx),
            lerp(c01.pet_mm, c11.pet_mm, fx),
            fz,
        );
        let t_min_c = lerp(
            lerp(c00.t_min_c, c10.t_min_c, fx),
            lerp(c01.t_min_c, c11.t_min_c, fx),
            fz,
        );
        if !p_mm.is_finite()
            || !pet_mm.is_finite()
            || p_mm < 0.0
            || pet_mm < 0.0
            || !t_min_c.is_finite()
        {
            return Err(RiverNetError::InvalidClimate { cell: (i, j) });
        }
        Ok(RunoffClimate {
            p_mm,
            pet_mm,
            t_min_c,
        })
    }

    pub const fn step_l(&self) -> f64 {
        self.step_l
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }
}

/// Один узел климатической решётки. Вынесена, чтобы последовательный и
/// параллельный пути считали ровно одну и ту же функцию точки.
fn climate_key_value<F>(key: Cell, step_l: f64, sample: &F) -> Result<RunoffClimate, RiverNetError>
where
    F: Fn(Vec2) -> Option<RunoffClimate> + Sync,
{
    let point = Vec2::new(key.0 as f64 * step_l, key.1 as f64 * step_l);
    let value = sample(point).ok_or(RiverNetError::InvalidClimate { cell: key })?;
    if !value.p_mm.is_finite()
        || !value.pet_mm.is_finite()
        || value.p_mm < 0.0
        || value.pet_mm < 0.0
        || !value.t_min_c.is_finite()
    {
        return Err(RiverNetError::InvalidClimate { cell: key });
    }
    Ok(value)
}

/// T79, вторая часть: 400+ узлов климата — почти всё время сборки сети рек
/// (180 мс из 212 на сиде 42 в gameplay). Узлы независимы, формула чистая,
/// поэтому делим отсортированный список ключей на чанки и считаем их в
/// `std::thread::scope`. Значения возвращаются в порядке ключей: ответ
/// побитово тот же, что при последовательном обходе. `LITHO_CLIMATE_GRID_SERIAL=1`
/// выключает параллелизм для диагностики и сверки.
fn sample_climate_keys<F>(
    keys: &[Cell],
    step_l: f64,
    sample: &F,
) -> Result<Vec<RunoffClimate>, RiverNetError>
where
    F: Fn(Vec2) -> Option<RunoffClimate> + Sync,
{
    let serial = std::env::var_os("LITHO_CLIMATE_GRID_SERIAL").is_some();
    let threads = if serial {
        1
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(CLIMATE_PARALLEL_MAX_THREADS)
    };
    if threads < 2 || keys.len() < CLIMATE_PARALLEL_MIN_KEYS {
        return keys
            .iter()
            .copied()
            .map(|key| climate_key_value(key, step_l, sample))
            .collect();
    }
    let chunk = keys.len().div_ceil(threads);
    std::thread::scope(|scope| {
        let handles: Vec<_> = keys
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || {
                    part.iter()
                        .copied()
                        .map(|key| climate_key_value(key, step_l, sample))
                        .collect::<Result<Vec<_>, _>>()
                })
            })
            .collect();
        let mut values = Vec::with_capacity(keys.len());
        for handle in handles {
            match handle.join() {
                Ok(Ok(part)) => values.extend(part),
                Ok(Err(err)) => return Err(err),
                Err(_) => {
                    return Err(RiverNetError::InvalidParameters(
                        "поток выборки климатической решётки завершился с паникой",
                    ));
                }
            }
        }
        Ok(values)
    })
}

fn checked_cell(i: i64, j: i64, di: i64, dj: i64) -> Option<Cell> {
    Some((i.checked_add(di)?, j.checked_add(dj)?))
}

/// Куда заканчивается водный путь узла. `Flowing` — продолжение в другом
/// узле; озёрные узлы ссылаются на баланс впадины, а не на русло внутри неё.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlowTerminal {
    #[default]
    Flowing,
    Sea,
    Rift,
    Transform,
    Lake {
        depression: usize,
    },
    Endorheic {
        depression: usize,
    },
    /// Замкнутая впадина с избытком воды, но без топографического выхода. T113/T114
    /// должны связать её с соседней плитой или уточнить контур озера.
    ClosedSurplus {
        depression: usize,
    },
    /// Узел без выхода и без пути вниз. В связанной сети плиты не возникает:
    /// узлы кольца без выхода в неё не входят.
    Boundary,
    /// Шаг 2 плана (T113): вода передана сети соседней плиты в узле кольца.
    Handoff,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RunoffNode {
    pub p_mm: f64,
    pub pet_mm: f64,
    pub t_min_c: f64,
    /// Формула плана для суши: max(0, P - k_et·PET)·h².
    pub runoff_mm_l2: f64,
    /// Фактический приход в баланс: на суше равен runoff, на зеркале — P·h².
    pub local_supply_mm_l2: f64,
    /// Шаг 2 плана (T113): вода, пришедшая в узел из сетей соседних плит.
    pub inflow_mm_l2: f64,
    /// Накопленный расход после испарения вышестоящих озёр, мм·L²/год.
    pub discharge_mm_l2: f64,
    pub downstream: Option<usize>,
    pub terminal: FlowTerminal,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BasinBalance {
    pub depression: usize,
    pub incoming_mm_l2: f64,
    pub evaporation_demand_mm_l2: f64,
    pub evaporation_mm_l2: f64,
    pub outflow_mm_l2: f64,
    /// Избыток замкнутой впадины без выхода; не выдаётся за реку.
    pub storage_mm_l2: f64,
    pub endorheic: bool,
    pub outlet_node: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Runoff {
    nodes: Vec<RunoffNode>,
    basins: Vec<BasinBalance>,
    climate_nodes: usize,
    export_mm_l2: f64,
    evaporation_mm_l2: f64,
    storage_mm_l2: f64,
    inflow_mm_l2: f64,
    handoff_mm_l2: f64,
}

impl Runoff {
    pub fn calculate(
        nodes: &[DrainageNode],
        depressions: &[Depression],
        grid: &ClimateGrid,
        params: &Params,
    ) -> Result<Self, RiverNetError> {
        Self::calculate_with(nodes, depressions, grid, params, None, None)
    }

    /// Расход с притоком от соседних плит и, при необходимости, только по
    /// части сети. `inflow[k]` — вода, пришедшая в узел `k` извне, мм·L²/год.
    /// `mask` ограничивает расчёт: узлы вне маски не читают климат и не дают
    /// воды. Маска обязана быть замкнута вверх по течению
    /// ([`upstream_mask`]), тогда расход на её узлах равен полному.
    pub fn calculate_with(
        nodes: &[DrainageNode],
        depressions: &[Depression],
        grid: &ClimateGrid,
        params: &Params,
        inflow: Option<&[f64]>,
        mask: Option<&[bool]>,
    ) -> Result<Self, RiverNetError> {
        if inflow.is_some_and(|v| v.len() != nodes.len())
            || mask.is_some_and(|v| v.len() != nodes.len())
        {
            return Err(RiverNetError::BrokenInvariant {
                node: 0,
                reason: "приток или маска стока другой длины, чем сеть",
            });
        }
        let p = &params.drainage;
        if !p.et_factor.is_finite() || p.et_factor < 0.0 {
            return Err(RiverNetError::InvalidParameters(
                "et_factor должен быть неотрицательным",
            ));
        }
        let area = p.step_l * p.step_l;
        if !p.step_l.is_finite() || p.step_l <= 0.0 || !area.is_finite() || area <= 0.0 {
            return Err(RiverNetError::InvalidParameters(
                "площадь узла стока должна быть положительной",
            ));
        }
        let mut runoff_nodes = Vec::with_capacity(nodes.len());
        for (k, node) in nodes.iter().enumerate() {
            if mask.is_some_and(|mask| !mask[k]) {
                runoff_nodes.push(RunoffNode::default());
                continue;
            }
            let incoming = inflow.map_or(0.0, |v| v[k]);
            if !incoming.is_finite() || incoming < 0.0 {
                return Err(RiverNetError::BrokenInvariant {
                    node: k,
                    reason: "неконечный или отрицательный приток от соседей",
                });
            }
            let climate = grid.sample(node.point)?;
            let land = node.in_plate && node.route_km >= 0.0 && node.outlet != DrainageOutlet::Sea;
            let runoff = if land {
                (climate.p_mm - p.et_factor * climate.pet_mm).max(0.0) * area
            } else {
                0.0
            };
            let local_supply = if land && node.depression.is_some() {
                // На зеркале считаем дождь P и испарение PET отдельно, чтобы
                // не вычесть PET дважды (сухой сток + испарение озера).
                climate.p_mm * area
            } else {
                runoff
            };
            if !runoff.is_finite() || !local_supply.is_finite() {
                return Err(RiverNetError::InvalidClimate { cell: node.cell });
            }
            runoff_nodes.push(RunoffNode {
                p_mm: climate.p_mm,
                pet_mm: climate.pet_mm,
                t_min_c: climate.t_min_c,
                runoff_mm_l2: runoff,
                local_supply_mm_l2: local_supply,
                inflow_mm_l2: incoming,
                discharge_mm_l2: 0.0,
                downstream: None,
                terminal: FlowTerminal::Flowing,
            });
        }

        let graph = FlowGraph::build(nodes, depressions, &runoff_nodes, area)?;
        let (mut basins, discharge) = graph.accumulate(nodes.len())?;
        for (balance, depression) in basins.iter_mut().zip(depressions) {
            if depression.plate_node_count == 0 {
                balance.endorheic = false;
            }
        }
        for (k, node) in nodes.iter().enumerate() {
            runoff_nodes[k].discharge_mm_l2 = discharge[k];
            if let Some(d) = node.depression {
                let balance = basins[d];
                runoff_nodes[k].downstream = None;
                runoff_nodes[k].terminal = basin_terminal(balance);
                continue;
            }
            match node.downstream {
                Some(down) if down < nodes.len() => {
                    if let Some(d) = nodes[down].depression {
                        runoff_nodes[k].downstream = None;
                        runoff_nodes[k].terminal = basin_terminal(basins[d]);
                    } else {
                        runoff_nodes[k].downstream = Some(down);
                    }
                }
                Some(_) => {
                    return Err(RiverNetError::BrokenInvariant {
                        node: k,
                        reason: "индекс стока вне сети",
                    });
                }
                None => {
                    runoff_nodes[k].terminal = match node.outlet {
                        DrainageOutlet::Sea => FlowTerminal::Sea,
                        DrainageOutlet::Rift => FlowTerminal::Rift,
                        DrainageOutlet::Transform => FlowTerminal::Transform,
                        DrainageOutlet::Handoff => FlowTerminal::Handoff,
                        DrainageOutlet::None => FlowTerminal::Boundary,
                    };
                }
            }
        }
        let export_mm_l2 = nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.depression.is_none())
            .map(|(k, _)| match runoff_nodes[k].terminal {
                FlowTerminal::Sea
                | FlowTerminal::Rift
                | FlowTerminal::Transform
                | FlowTerminal::Handoff
                | FlowTerminal::Boundary => runoff_nodes[k].discharge_mm_l2,
                _ => 0.0,
            })
            .sum();
        let handoff_mm_l2 = runoff_nodes
            .iter()
            .filter(|node| node.terminal == FlowTerminal::Handoff)
            .map(|node| node.discharge_mm_l2)
            .sum();
        let inflow_mm_l2 = runoff_nodes.iter().map(|node| node.inflow_mm_l2).sum();
        let evaporation_mm_l2 = basins.iter().map(|b| b.evaporation_mm_l2).sum();
        let storage_mm_l2 = basins.iter().map(|b| b.storage_mm_l2).sum();
        let result = Self {
            nodes: runoff_nodes,
            basins,
            climate_nodes: grid.len(),
            export_mm_l2,
            evaporation_mm_l2,
            storage_mm_l2,
            inflow_mm_l2,
            handoff_mm_l2,
        };
        result.check_balance(nodes, area)?;
        Ok(result)
    }

    pub fn nodes(&self) -> &[RunoffNode] {
        &self.nodes
    }

    pub fn basins(&self) -> &[BasinBalance] {
        &self.basins
    }

    pub const fn climate_nodes(&self) -> usize {
        self.climate_nodes
    }

    pub const fn export_mm_l2(&self) -> f64 {
        self.export_mm_l2
    }

    pub const fn evaporation_mm_l2(&self) -> f64 {
        self.evaporation_mm_l2
    }

    pub const fn storage_mm_l2(&self) -> f64 {
        self.storage_mm_l2
    }

    /// Шаг 2 плана (T113): вода, пришедшая из сетей соседних плит.
    pub const fn inflow_mm_l2(&self) -> f64 {
        self.inflow_mm_l2
    }

    /// Шаг 2 плана (T113): вода, переданная сетям соседних плит. Входит в
    /// `export_mm_l2`.
    pub const fn handoff_mm_l2(&self) -> f64 {
        self.handoff_mm_l2
    }

    pub fn endorheic_count(&self) -> usize {
        self.basins.iter().filter(|b| b.endorheic).count()
    }

    fn check_balance(&self, nodes: &[DrainageNode], area: f64) -> Result<(), RiverNetError> {
        if self.nodes.len() != nodes.len() {
            return Err(RiverNetError::BrokenInvariant {
                node: 0,
                reason: "длина климатического стока не совпадает с сетью",
            });
        }
        let supply: f64 = self
            .nodes
            .iter()
            .map(|n| n.local_supply_mm_l2 + n.inflow_mm_l2)
            .sum();
        let accounted = self.export_mm_l2 + self.evaporation_mm_l2 + self.storage_mm_l2;
        let tolerance = 1e-9 * supply.abs().max(area).max(1.0);
        if !supply.is_finite() || !accounted.is_finite() || (supply - accounted).abs() > tolerance {
            return Err(RiverNetError::BrokenInvariant {
                node: 0,
                reason: "годовой баланс стока не сходится",
            });
        }
        for (k, node) in self.nodes.iter().enumerate() {
            if !node.p_mm.is_finite()
                || !node.pet_mm.is_finite()
                || !node.t_min_c.is_finite()
                || !node.runoff_mm_l2.is_finite()
                || !node.local_supply_mm_l2.is_finite()
                || !node.inflow_mm_l2.is_finite()
                || !node.discharge_mm_l2.is_finite()
                || node.p_mm < 0.0
                || node.pet_mm < 0.0
                || node.runoff_mm_l2 < 0.0
                || node.local_supply_mm_l2 < 0.0
                || node.discharge_mm_l2 < 0.0
            {
                return Err(RiverNetError::BrokenInvariant {
                    node: k,
                    reason: "неконечный или отрицательный расход",
                });
            }
        }
        Ok(())
    }
}

fn basin_terminal(balance: BasinBalance) -> FlowTerminal {
    // Принадлежность узлов не меняет водный путь: чужая впадина тоже может
    // пропустить воду дальше. Boundary здесь удвоил бы экспорт (вход и выход).
    if balance.endorheic {
        FlowTerminal::Endorheic {
            depression: balance.depression,
        }
    } else if balance.outlet_node.is_some() {
        FlowTerminal::Lake {
            depression: balance.depression,
        }
    } else {
        FlowTerminal::ClosedSurplus {
            depression: balance.depression,
        }
    }
}

/// Шаг 2 плана (T113): узлы, вода которых приходит в `targets`, и сами
/// `targets`. Впадина входит целиком: она считается одним резервуаром.
/// Расход на узлах маски зависит только от узлов маски.
pub fn upstream_mask(
    nodes: &[DrainageNode],
    depressions: &[Depression],
    targets: &[usize],
) -> Result<Vec<bool>, RiverNetError> {
    let zero = vec![RunoffNode::default(); nodes.len()];
    let graph = FlowGraph::build(nodes, depressions, &zero, 1.0)?;
    let mut upstream = vec![Vec::new(); graph.units.len()];
    for (k, unit) in graph.units.iter().enumerate() {
        if let Some(down) = unit.downstream {
            upstream[down].push(k);
        }
    }
    let mut reached = vec![false; graph.units.len()];
    let mut stack = Vec::new();
    for &target in targets {
        let Some(&unit) = graph.node_unit.get(target) else {
            return Err(RiverNetError::BrokenInvariant {
                node: target,
                reason: "цель маски стока вне сети",
            });
        };
        if !reached[unit] {
            reached[unit] = true;
            stack.push(unit);
        }
    }
    while let Some(unit) = stack.pop() {
        for &up in &upstream[unit] {
            if !reached[up] {
                reached[up] = true;
                stack.push(up);
            }
        }
    }
    Ok(graph.node_unit.iter().map(|&unit| reached[unit]).collect())
}

/// Временный граф водных балансов: обычный узел или целая впадина.
/// Впадина стягивается в один резервуар, поэтому испарение считается один раз.
struct FlowGraph {
    units: Vec<FlowUnit>,
    node_unit: Vec<usize>,
    incoming: Vec<f64>,
    indegree: Vec<usize>,
}

struct FlowUnit {
    control_km: f64,
    supply: f64,
    evaporation: f64,
    downstream: Option<usize>,
    node: Option<usize>,
    depression: Option<usize>,
    exit_node: Option<usize>,
}

impl FlowGraph {
    fn build(
        nodes: &[DrainageNode],
        depressions: &[Depression],
        runoff: &[RunoffNode],
        area: f64,
    ) -> Result<Self, RiverNetError> {
        let n = nodes.len();
        let mut units = Vec::with_capacity(n + depressions.len());
        let mut node_unit = vec![0; n];
        for (k, node) in nodes.iter().enumerate() {
            node_unit[k] = match node.depression {
                Some(d) if d < depressions.len() => n + d,
                Some(_) => {
                    return Err(RiverNetError::BrokenInvariant {
                        node: k,
                        reason: "индекс впадины вне сети",
                    });
                }
                None => k,
            };
            units.push(FlowUnit {
                control_km: node.filled_km,
                supply: 0.0,
                evaporation: 0.0,
                downstream: None,
                node: Some(k),
                depression: None,
                exit_node: None,
            });
        }
        for (d, depression) in depressions.iter().enumerate() {
            if depression.bottom_node >= n {
                return Err(RiverNetError::BrokenInvariant {
                    node: 0,
                    reason: "дно впадины вне сети",
                });
            }
            units.push(FlowUnit {
                control_km: nodes[depression.bottom_node].filled_km,
                supply: 0.0,
                evaporation: 0.0,
                downstream: None,
                node: None,
                depression: Some(d),
                exit_node: None,
            });
        }
        for (k, node) in nodes.iter().enumerate() {
            let unit = node_unit[k];
            units[unit].supply += runoff[k].local_supply_mm_l2 + runoff[k].inflow_mm_l2;
            if node.depression.is_some() && node.in_plate {
                units[unit].evaporation += runoff[k].pet_mm * area;
            }
            match node.depression {
                None => {
                    if let Some(down) = node.downstream {
                        if down >= n {
                            return Err(RiverNetError::BrokenInvariant {
                                node: k,
                                reason: "индекс стока вне сети",
                            });
                        }
                        units[unit].downstream = Some(node_unit[down]);
                    }
                }
                Some(d) => {
                    if let Some(down) = node.downstream {
                        if down >= n || nodes[down].depression != Some(d) {
                            let exit = n + d;
                            let better = units[exit].exit_node.is_none_or(|old| {
                                (node.filled_km, k) < (nodes[old].filled_km, old)
                            });
                            if better {
                                units[exit].exit_node = Some(k);
                                units[exit].control_km = node.filled_km;
                                units[exit].downstream = Some(node_unit[down]);
                            }
                        }
                    }
                }
            }
        }
        let mut indegree = vec![0; units.len()];
        for (k, unit) in units.iter().enumerate() {
            if let Some(down) = unit.downstream {
                if down >= units.len() || down == k {
                    return Err(RiverNetError::BrokenInvariant {
                        node: 0,
                        reason: "неверное ребро водного графа",
                    });
                }
                indegree[down] += 1;
            }
        }
        Ok(Self {
            units,
            node_unit,
            incoming: vec![0.0; n + depressions.len()],
            indegree,
        })
    }

    fn accumulate(
        mut self,
        node_count: usize,
    ) -> Result<(Vec<BasinBalance>, Vec<f64>), RiverNetError> {
        let mut ready = BinaryHeap::new();
        for (k, unit) in self.units.iter().enumerate() {
            if self.indegree[k] == 0 {
                ready.push(ReadyUnit {
                    control_km: unit.control_km,
                    unit: k,
                });
            }
        }
        let mut basins = vec![
            BasinBalance {
                depression: 0,
                incoming_mm_l2: 0.0,
                evaporation_demand_mm_l2: 0.0,
                evaporation_mm_l2: 0.0,
                outflow_mm_l2: 0.0,
                storage_mm_l2: 0.0,
                endorheic: false,
                outlet_node: None,
            };
            self.units.len() - node_count
        ];
        let mut discharge = vec![0.0; self.units.len()];
        let mut processed = 0;
        while let Some(entry) = ready.pop() {
            let k = entry.unit;
            let unit = &self.units[k];
            let incoming = unit.supply + self.incoming[k];
            let (outflow, balance) = if let Some(d) = unit.depression {
                let evaporation = incoming.min(unit.evaporation);
                let surplus = incoming - evaporation;
                let has_exit = unit.downstream.is_some();
                let outflow = if has_exit { surplus } else { 0.0 };
                let storage = if has_exit { 0.0 } else { surplus };
                let endorheic = surplus <= 0.0;
                (
                    outflow,
                    Some(BasinBalance {
                        depression: d,
                        incoming_mm_l2: incoming,
                        evaporation_demand_mm_l2: unit.evaporation,
                        evaporation_mm_l2: evaporation,
                        outflow_mm_l2: outflow,
                        storage_mm_l2: storage,
                        endorheic,
                        outlet_node: unit.exit_node,
                    }),
                )
            } else {
                (incoming, None)
            };
            if let Some(node) = unit.node {
                discharge[node] = outflow;
            } else if let Some(d) = unit.depression {
                basins[d] = balance.expect("баланс впадины рассчитан");
            }
            if let Some(down) = unit.downstream {
                self.incoming[down] += outflow;
                self.indegree[down] -= 1;
                if self.indegree[down] == 0 {
                    ready.push(ReadyUnit {
                        control_km: self.units[down].control_km,
                        unit: down,
                    });
                }
            }
            processed += 1;
        }
        if processed != self.units.len() {
            return Err(RiverNetError::BrokenInvariant {
                node: 0,
                reason: "цикл в графе стока",
            });
        }
        for (k, node) in discharge.iter_mut().enumerate().take(node_count) {
            let unit = self.node_unit[k];
            if self.units[unit].depression.is_some() {
                let d = self.units[unit].depression.expect("id впадины");
                *node = basins[d].outflow_mm_l2;
            }
        }
        Ok((basins, discharge))
    }
}

struct ReadyUnit {
    control_km: f64,
    unit: usize,
}

impl PartialEq for ReadyUnit {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for ReadyUnit {}

impl Ord for ReadyUnit {
    fn cmp(&self, other: &Self) -> Ordering {
        // Среди топологически готовых узлов — по убыванию f, ничьи по номеру.
        self.control_km
            .total_cmp(&other.control_km)
            .then_with(|| other.unit.cmp(&self.unit))
    }
}

impl PartialOrd for ReadyUnit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::{ClimateGrid, FlowTerminal, Runoff, RunoffClimate, upstream_mask};
    use crate::drainage::{Depression, DepressionKind, DrainageNode, DrainageOutlet};
    use crate::math::Vec2;
    use crate::mode::Mode;
    use crate::params::Params;
    use crate::version::GenVersion;

    fn params() -> Params {
        Params::defaults(GenVersion::V1Dev, Mode::Gameplay)
    }

    fn node(i: i64, height: f64, down: Option<usize>, outlet: DrainageOutlet) -> DrainageNode {
        DrainageNode {
            cell: (i, 0),
            point: Vec2::new(i as f64 * 0.25, 0.0),
            in_plate: true,
            route_km: height,
            filled_km: height,
            downstream: down,
            outlet,
            depression: None,
        }
    }

    fn grid(points: &[Vec2], p: f64, pet: f64) -> ClimateGrid {
        ClimateGrid::build(points, 1.0, |_| {
            Some(RunoffClimate {
                p_mm: p,
                pet_mm: pet,
                t_min_c: 10.0,
            })
        })
        .unwrap()
    }

    /// T79, вторая часть: попадаем в параллельную ветку (`>= 64` ключей) и
    /// сверяем каждый узел с чистой функцией точки. Порядок обхода ключей и
    /// ответ не должны зависеть от числа потоков.
    #[test]
    fn climate_grid_parallel_matches_pointwise_values() {
        let points: Vec<Vec2> = (0..12)
            .flat_map(|i| (0..12).map(move |j| Vec2::new(i as f64 * 1.7, j as f64 * 1.7)))
            .collect();
        let grid = ClimateGrid::build(&points, 1.0, |q| {
            Some(RunoffClimate {
                p_mm: 100.0 + 0.5 * q.x,
                pet_mm: 20.0 + 0.25 * q.z,
                t_min_c: 5.0 + q.x - q.z,
            })
        })
        .unwrap();
        assert!(grid.len() >= super::CLIMATE_PARALLEL_MIN_KEYS);
        for (&key, value) in &grid.cells {
            let q = Vec2::new(key.0 as f64, key.1 as f64);
            assert_eq!(value.p_mm, 100.0 + 0.5 * q.x);
            assert_eq!(value.pet_mm, 20.0 + 0.25 * q.z);
            assert_eq!(value.t_min_c, 5.0 + q.x - q.z);
        }
    }

    #[test]
    fn bilinear_grid_is_exact_on_linear_field_and_handles_negative_coords() {
        let points = [
            Vec2::new(-0.75, -0.25),
            Vec2::new(0.75, 0.25),
            Vec2::new(-0.75, 0.75),
        ];
        let grid = ClimateGrid::build(&points, 1.0, |q| {
            Some(RunoffClimate {
                p_mm: 100.0 + 10.0 * q.x,
                pet_mm: 20.0 + 2.0 * q.z,
                t_min_c: 5.0 + q.x - q.z,
            })
        })
        .unwrap();
        let got = grid.sample(Vec2::new(-0.25, 0.5)).unwrap();
        assert!((got.p_mm - 97.5).abs() < 1e-12);
        assert!((got.pet_mm - 21.0).abs() < 1e-12);
        assert!((got.t_min_c - 4.25).abs() < 1e-12);
    }

    #[test]
    fn discharge_accumulates_downhill_and_dry_nodes_stay_zero() {
        let p = params();
        let nodes = vec![
            node(3, 3.0, Some(1), DrainageOutlet::None),
            node(2, 2.0, Some(2), DrainageOutlet::None),
            node(1, 1.0, None, DrainageOutlet::Sea),
            node(0, 4.0, None, DrainageOutlet::None),
        ];
        // Последний узел ring: передаёт воду, но не добавляет сток.
        let mut nodes = nodes;
        nodes[3].in_plate = false;
        let points: Vec<_> = nodes.iter().map(|n| n.point).collect();
        let g = grid(&points, 100.0, 0.0);
        let runoff = Runoff::calculate(&nodes, &[], &g, &p).unwrap();
        let q = 100.0 * 0.25 * 0.25;
        assert_eq!(runoff.nodes()[0].discharge_mm_l2, q);
        assert_eq!(runoff.nodes()[1].discharge_mm_l2, 2.0 * q);
        assert_eq!(runoff.nodes()[2].discharge_mm_l2, 2.0 * q);
        assert_eq!(runoff.nodes()[2].terminal, FlowTerminal::Sea);
        assert_eq!(runoff.nodes()[3].discharge_mm_l2, 0.0);
        let dry_points: Vec<_> = nodes.iter().map(|n| n.point).collect();
        let dry = Runoff::calculate(&nodes, &[], &grid(&dry_points, 10.0, 100.0), &p).unwrap();
        assert!(dry.nodes().iter().all(|n| n.discharge_mm_l2 == 0.0));
    }

    #[test]
    fn neighbour_inflow_is_added_and_handed_on_with_balance() {
        // Шаг 2 плана (T113): 0 → 1 → 2 (передача соседу), 3 → 4 (море).
        let p = params();
        let mut nodes = vec![
            node(4, 3.0, Some(1), DrainageOutlet::None),
            node(3, 2.0, Some(2), DrainageOutlet::None),
            node(2, 1.0, None, DrainageOutlet::Handoff),
            node(1, 2.0, Some(4), DrainageOutlet::None),
            node(0, 0.0, None, DrainageOutlet::Sea),
        ];
        nodes[2].in_plate = false;
        let points: Vec<_> = nodes.iter().map(|n| n.point).collect();
        let g = grid(&points, 100.0, 0.0);
        let q = 100.0 * 0.25 * 0.25;
        let inflow = [0.0, 40.0, 0.0, 0.0, 0.0];
        let full = Runoff::calculate_with(&nodes, &[], &g, &p, Some(&inflow), None).unwrap();
        assert_eq!(full.nodes()[0].discharge_mm_l2, q);
        assert_eq!(full.nodes()[1].discharge_mm_l2, 2.0 * q + 40.0);
        assert_eq!(full.nodes()[2].discharge_mm_l2, 2.0 * q + 40.0);
        assert_eq!(full.nodes()[2].terminal, FlowTerminal::Handoff);
        assert_eq!(full.inflow_mm_l2(), 40.0);
        assert_eq!(full.handoff_mm_l2(), 2.0 * q + 40.0);
        assert_eq!(full.export_mm_l2(), 3.0 * q + 40.0);

        // Маска водосбора узла передачи: расход в ней тот же, остальное не
        // считается и климата не требует.
        let mask = upstream_mask(&nodes, &[], &[2]).unwrap();
        assert_eq!(mask, [true, true, true, false, false]);
        let part_points: Vec<_> = points[..3].to_vec();
        let part_grid = grid(&part_points, 100.0, 0.0);
        let part = Runoff::calculate_with(&nodes, &[], &part_grid, &p, Some(&inflow), Some(&mask))
            .unwrap();
        for k in 0..3 {
            assert_eq!(
                part.nodes()[k].discharge_mm_l2,
                full.nodes()[k].discharge_mm_l2
            );
        }
        assert_eq!(part.nodes()[3].discharge_mm_l2, 0.0);
        assert!(upstream_mask(&nodes, &[], &[9]).is_err());
    }

    fn lake_nodes() -> (Vec<DrainageNode>, Vec<Depression>) {
        let mut nodes = vec![
            node(4, 4.0, Some(1), DrainageOutlet::None),
            node(3, 1.0, Some(3), DrainageOutlet::None),
            node(2, 0.5, Some(1), DrainageOutlet::None),
            node(1, 0.0, None, DrainageOutlet::Sea),
        ];
        nodes[2].filled_km = 1.0001;
        nodes[1].depression = Some(0);
        nodes[2].depression = Some(0);
        let depressions = vec![Depression {
            kind: DepressionKind::Filled { spill_km: 1.01 },
            bottom_node: 2,
            node_count: 2,
            plate_node_count: 2,
            max_depth_km: 0.51,
        }];
        (nodes, depressions)
    }

    #[test]
    fn wet_lake_evaporates_once_and_exports_surplus() {
        let p = params();
        let (nodes, depressions) = lake_nodes();
        let points: Vec<_> = nodes.iter().map(|n| n.point).collect();
        let g = grid(&points, 100.0, 10.0);
        let runoff = Runoff::calculate(&nodes, &depressions, &g, &p).unwrap();
        let area = 0.25 * 0.25;
        // Upstream runoff 90a + rain on two lake cells 200a - PET 20a.
        let expected = (90.0 + 200.0 - 20.0) * area;
        let balance = runoff.basins()[0];
        assert!((balance.outflow_mm_l2 - expected).abs() < 1e-12);
        assert!(!balance.endorheic);
        assert_eq!(
            runoff.nodes()[1].terminal,
            FlowTerminal::Lake { depression: 0 }
        );
        assert_eq!(
            runoff.nodes()[0].terminal,
            FlowTerminal::Lake { depression: 0 }
        );
        assert!((runoff.export_mm_l2() - expected).abs() < 1e-12);
    }

    #[test]
    fn dry_lake_is_endorheic_and_does_not_export_water() {
        let p = params();
        let (nodes, depressions) = lake_nodes();
        let points: Vec<_> = nodes.iter().map(|n| n.point).collect();
        let g = grid(&points, 10.0, 100.0);
        let runoff = Runoff::calculate(&nodes, &depressions, &g, &p).unwrap();
        let balance = runoff.basins()[0];
        assert!(balance.endorheic);
        assert_eq!(balance.outflow_mm_l2, 0.0);
        assert_eq!(runoff.export_mm_l2(), 0.0);
        assert_eq!(
            runoff.nodes()[0].terminal,
            FlowTerminal::Endorheic { depression: 0 }
        );
    }
}

#[cfg(test)]
mod balance_tests {
    use super::{ClimateGrid, FlowTerminal, Runoff, RunoffClimate};
    use crate::drainage::{Depression, DepressionKind, DrainageNode, DrainageOutlet};
    use crate::math::Vec2;
    use crate::mode::Mode;
    use crate::params::Params;
    use crate::version::GenVersion;

    #[test]
    fn closed_wet_basin_keeps_surplus_in_storage_not_a_fake_river() {
        let params = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        let nodes = vec![
            DrainageNode {
                cell: (0, 0),
                point: Vec2::new(0.0, 0.0),
                in_plate: true,
                route_km: 1.0,
                filled_km: 1.0,
                downstream: None,
                outlet: DrainageOutlet::None,
                depression: Some(0),
            },
            DrainageNode {
                cell: (1, 0),
                point: Vec2::new(0.25, 0.0),
                in_plate: true,
                route_km: 2.0,
                filled_km: 2.0,
                downstream: None,
                outlet: DrainageOutlet::None,
                depression: Some(0),
            },
        ];
        let depressions = vec![Depression {
            kind: DepressionKind::Closed,
            bottom_node: 0,
            node_count: 2,
            plate_node_count: 2,
            max_depth_km: 0.0,
        }];
        let points: Vec<_> = nodes.iter().map(|n| n.point).collect();
        let grid = ClimateGrid::build(&points, 1.0, |_| {
            Some(RunoffClimate {
                p_mm: 100.0,
                pet_mm: 10.0,
                t_min_c: 10.0,
            })
        })
        .unwrap();
        let runoff = Runoff::calculate(&nodes, &depressions, &grid, &params).unwrap();
        let area = 0.25 * 0.25;
        let balance = runoff.basins()[0];
        assert!(!balance.endorheic);
        assert_eq!(balance.outlet_node, None);
        assert_eq!(balance.outflow_mm_l2, 0.0);
        assert!((balance.storage_mm_l2 - 180.0 * area).abs() < 1e-12);
        assert_eq!(runoff.export_mm_l2(), 0.0);
        assert_eq!(
            runoff.nodes()[0].terminal,
            FlowTerminal::ClosedSurplus { depression: 0 }
        );
    }

    #[test]
    fn foreign_basin_transmits_water_without_double_export() {
        let params = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        let node = |cell: (i64, i64),
                    in_plate: bool,
                    route_km: f64,
                    downstream: Option<usize>,
                    outlet: DrainageOutlet,
                    depression: Option<usize>| DrainageNode {
            cell,
            point: Vec2::new(cell.0 as f64 * 0.25, cell.1 as f64 * 0.25),
            in_plate,
            route_km,
            filled_km: route_km,
            downstream,
            outlet,
            depression,
        };
        let nodes = vec![
            node((0, 0), true, 2.0, Some(1), DrainageOutlet::None, None),
            node((1, 0), false, 1.0, Some(2), DrainageOutlet::None, Some(0)),
            node((2, 0), false, 0.0, None, DrainageOutlet::Transform, None),
        ];
        let depressions = vec![Depression {
            kind: DepressionKind::Filled { spill_km: 1.0 },
            bottom_node: 1,
            node_count: 1,
            plate_node_count: 0,
            max_depth_km: 0.1,
        }];
        let points: Vec<_> = nodes.iter().map(|n| n.point).collect();
        let grid = ClimateGrid::build(&points, 1.0, |_| {
            Some(RunoffClimate {
                p_mm: 400.0,
                pet_mm: 100.0,
                t_min_c: 10.0,
            })
        })
        .unwrap();
        let runoff = Runoff::calculate(&nodes, &depressions, &grid, &params).unwrap();
        let supply = (400.0 - 100.0) * 0.25 * 0.25;
        assert_eq!(
            runoff.nodes()[0].terminal,
            FlowTerminal::Lake { depression: 0 }
        );
        assert!((runoff.export_mm_l2() - supply).abs() < 1e-12);
        assert_eq!(runoff.evaporation_mm_l2(), 0.0);
        assert_eq!(runoff.storage_mm_l2(), 0.0);
        assert!((runoff.basins()[0].outflow_mm_l2 - supply).abs() < 1e-12);
    }

    #[test]
    fn invalid_climate_is_rejected_not_replaced_by_zero() {
        let points = [Vec2::new(0.0, 0.0)];
        assert!(ClimateGrid::build(&points, 1.0, |_| None).is_err());
        assert!(
            ClimateGrid::build(&points, 1.0, |_| Some(RunoffClimate {
                p_mm: -1.0,
                pet_mm: 0.0,
                t_min_c: 10.0,
            }))
            .is_err()
        );
    }
}
