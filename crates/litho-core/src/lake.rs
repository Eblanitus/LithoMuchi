//! Озёра и котловины поверх впадин T110 и баланса T111 (PLAN-REVISION 5.6, T114).
//!
//! Контур строится по затопленным клеткам решётки стока, а не по шумовой
//! «луже». Уровень перелива берётся с контролирующего порога рельефа, без
//! ε-наклона priority-flood. Замкнутая впадина с избытком не получает
//! фиктивного перелива: её уровень оставлен T145. Высокие озёра не отсекаются
//! потолком высоты (R2 решён автором), но в игру не попадают до T115/T143.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::channel::BASKET_L;
use crate::drainage::{Depression, DepressionKind, DrainageNode, DrainageOutlet, RiverNetError};
use crate::math::{Vec2, clamp, smoothstep};
use crate::params::Params;
use crate::runoff::Runoff;

type Cell = (i64, i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LakeKind {
    Lake,
    InlandSea,
    Swamp,
    DryBasin,
    /// Годовой избыток есть, но топографического перелива нет. Не выдаём
    /// annual storage за уровень озера; контур влияния есть, воды пока нет.
    ClosedSurplus,
    /// Дно ниже моря и связано с морским выходом: это не пресное озеро.
    SeaConnected,
}

impl LakeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lake => "lake",
            Self::InlandSea => "inland_sea",
            Self::Swamp => "swamp",
            Self::DryBasin => "dry_basin",
            Self::ClosedSurplus => "closed_surplus",
            Self::SeaConnected => "sea_connected",
        }
    }

    pub const fn has_water(self) -> bool {
        matches!(
            self,
            Self::Lake | Self::InlandSea | Self::Swamp | Self::SeaConnected
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LakeSurface {
    pub depression: usize,
    pub kind: LakeKind,
    /// Замкнутые кольца без повторения последней точки. Первое обычно внешнее,
    /// остальные — острова; запрос использует чётность пересечений.
    pub rings: Vec<Vec<Vec2>>,
    pub center: Vec2,
    pub basin_area_l2: f64,
    pub water_area_l2: f64,
    pub owned_water_area_l2: f64,
    pub level_km: Option<f64>,
    pub max_depth_km: f64,
    pub shore_l: f64,
    pub frozen: bool,
    pub rift: bool,
    /// R2 потребуется для воды выше моря; это не запрет озера в v1.
    pub high: bool,
    bounds: (Vec2, Vec2),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LakeHit {
    pub plate_id: u64,
    pub lake: usize,
    pub depression: usize,
    pub kind: LakeKind,
    pub distance_l: f64,
    pub inside: bool,
    pub level_km: Option<f64>,
    pub max_depth_km: f64,
    pub frozen: bool,
    pub rift: bool,
    pub high: bool,
    pub water_strength: f64,
    pub shore_strength: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LakeQuery {
    pub water: f64,
    pub shore: f64,
    pub basin: f64,
    pub nearest: Option<LakeHit>,
    pub networks_considered: usize,
}

impl LakeQuery {
    pub fn merge(&mut self, other: Self) {
        self.water = self.water.max(other.water);
        self.shore = self.shore.max(other.shore);
        self.basin = self.basin.max(other.basin);
        self.networks_considered += other.networks_considered;
        if let Some(hit) = other.nearest {
            if self.nearest.is_none_or(|old| {
                hit.distance_l.total_cmp(&old.distance_l).then_with(|| {
                    (hit.plate_id, hit.lake, hit.depression).cmp(&(
                        old.plate_id,
                        old.lake,
                        old.depression,
                    ))
                }) == std::cmp::Ordering::Less
            }) {
                self.nearest = Some(hit);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LakeNetwork {
    plate_id: u64,
    lakes: Vec<LakeSurface>,
    baskets: HashMap<Cell, Vec<usize>>,
}

impl LakeNetwork {
    pub fn build(
        plate_id: u64,
        nodes: &[DrainageNode],
        depressions: &[Depression],
        runoff: &Runoff,
        params: &Params,
    ) -> Result<Self, RiverNetError> {
        validate(params)?;
        if runoff.nodes().len() != nodes.len() || runoff.basins().len() != depressions.len() {
            return Err(RiverNetError::BrokenInvariant {
                node: 0,
                reason: "озёра: длины стока, впадин и узлов не совпадают",
            });
        }
        let h = params.drainage.step_l;
        let area = h * h;
        let shore_l = params.lakes.shore_fraction * h;
        let index: HashMap<Cell, usize> = nodes
            .iter()
            .enumerate()
            .map(|(k, node)| (node.cell, k))
            .collect();
        let mut members = vec![Vec::new(); depressions.len()];
        for (k, node) in nodes.iter().enumerate() {
            if let Some(d) = node.depression {
                if d >= depressions.len() {
                    return Err(RiverNetError::BrokenInvariant {
                        node: k,
                        reason: "озёра: индекс впадины вне сети",
                    });
                }
                members[d].push(k);
            }
        }
        let mut lakes = Vec::new();
        for (d, depression) in depressions.iter().enumerate() {
            if depression.plate_node_count == 0 || members[d].is_empty() {
                continue;
            }
            let balance = runoff.basins()[d];
            let wet_supply = balance.outflow_mm_l2 + balance.storage_mm_l2;
            let saddle = match depression.kind {
                DepressionKind::Filled { .. } => {
                    Some(spill_level(nodes, &members[d], d, depression.bottom_node)?)
                }
                DepressionKind::Closed => None,
            };
            // Морская связь ниже нуля не зависит от пресного годового баланса.
            let level = match saddle {
                Some(level) if level <= 0.0 => Some(level),
                Some(level) if !balance.endorheic && wet_supply > 0.0 => Some(level),
                _ => None,
            };
            let mut mask = vec![false; nodes.len()];
            let mut water_nodes = Vec::new();
            if let Some(level) = level {
                let water_level = if level <= 0.0 { 0.0 } else { level };
                let mut stack = vec![depression.bottom_node];
                if nodes[depression.bottom_node].route_km <= water_level {
                    mask[depression.bottom_node] = true;
                }
                while let Some(k) = stack.pop() {
                    if !mask[k] {
                        continue;
                    }
                    water_nodes.push(k);
                    for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let (Some(i), Some(j)) = (
                            nodes[k].cell.0.checked_add(di),
                            nodes[k].cell.1.checked_add(dj),
                        ) else {
                            continue;
                        };
                        let cell = (i, j);
                        let Some(&next) = index.get(&cell) else {
                            continue;
                        };
                        if !mask[next]
                            && nodes[next].depression == Some(d)
                            && nodes[next].route_km <= water_level
                        {
                            mask[next] = true;
                            stack.push(next);
                        }
                    }
                }
                water_nodes.sort_unstable();
            }
            let has_water = !water_nodes.is_empty();
            if !has_water {
                for &k in &members[d] {
                    mask[k] = true;
                }
            }
            let water_area_l2 = water_nodes.len() as f64 * area;
            let owned_water_area_l2 =
                water_nodes.iter().filter(|&&k| nodes[k].in_plate).count() as f64 * area;
            let depth_km = level
                .map(|level| {
                    water_nodes
                        .iter()
                        .map(|&k| (level.max(0.0) - nodes[k].route_km).max(0.0))
                        .fold(0.0_f64, f64::max)
                })
                .unwrap_or(0.0);
            let climate_nodes = if has_water { &water_nodes } else { &members[d] };
            let mut p_sum = 0.0;
            let mut pet_sum = 0.0;
            let mut t_sum = 0.0;
            let mut climate_count = 0.0;
            for &k in climate_nodes {
                if nodes[k].in_plate {
                    p_sum += runoff.nodes()[k].p_mm;
                    pet_sum += runoff.nodes()[k].pet_mm;
                    t_sum += runoff.nodes()[k].t_min_c;
                    climate_count += 1.0;
                }
            }
            let humid = climate_count > 0.0 && p_sum >= params.lakes.swamp_min_p_pet * pet_sum;
            let frozen =
                climate_count > 0.0 && t_sum / climate_count <= params.lakes.frozen_t_min_c;
            let rift = members[d].iter().any(|&k| {
                nodes[k].outlet == DrainageOutlet::Rift
                    || nodes[k]
                        .downstream
                        .is_some_and(|down| nodes[down].outlet == DrainageOutlet::Rift)
            });
            let kind = if !has_water {
                if matches!(depression.kind, DepressionKind::Closed) && balance.storage_mm_l2 > 0.0
                {
                    LakeKind::ClosedSurplus
                } else {
                    LakeKind::DryBasin
                }
            } else if level.is_some_and(|level| level <= 0.0) {
                LakeKind::SeaConnected
            } else if depth_km <= params.lakes.swamp_max_depth_km && humid {
                LakeKind::Swamp
            } else if water_area_l2 > params.lakes.inland_sea_area_l2 {
                LakeKind::InlandSea
            } else {
                LakeKind::Lake
            };
            let rings = contours(nodes, &mask, h)?;
            let points: Vec<Vec2> = rings.iter().flatten().copied().collect();
            let (Some(min_x), Some(max_x), Some(min_z), Some(max_z)) = (
                points.iter().map(|p| p.x).reduce(f64::min),
                points.iter().map(|p| p.x).reduce(f64::max),
                points.iter().map(|p| p.z).reduce(f64::min),
                points.iter().map(|p| p.z).reduce(f64::max),
            ) else {
                return Err(RiverNetError::BrokenInvariant {
                    node: depression.bottom_node,
                    reason: "озеро без контура",
                });
            };
            let center = if has_water {
                let sum = water_nodes
                    .iter()
                    .fold(Vec2::default(), |acc, &k| acc.add(nodes[k].point));
                sum.scale(1.0 / water_nodes.len() as f64)
            } else {
                nodes[depression.bottom_node].point
            };
            lakes.push(LakeSurface {
                depression: d,
                kind,
                rings,
                center,
                basin_area_l2: members[d].len() as f64 * area,
                water_area_l2,
                owned_water_area_l2,
                level_km: level.map(|level| level.max(0.0)),
                max_depth_km: depth_km,
                shore_l,
                frozen: frozen && kind.has_water(),
                rift,
                high: level.is_some_and(|level| level > 0.0),
                bounds: (Vec2::new(min_x, min_z), Vec2::new(max_x, max_z)),
            });
        }
        let mut baskets = HashMap::<Cell, Vec<usize>>::new();
        for (k, lake) in lakes.iter().enumerate() {
            let lo = lake.bounds.0;
            let hi = lake.bounds.1;
            let i0 = ((lo.x - lake.shore_l) / BASKET_L).floor() as i64;
            let i1 = ((hi.x + lake.shore_l) / BASKET_L).floor() as i64;
            let j0 = ((lo.z - lake.shore_l) / BASKET_L).floor() as i64;
            let j1 = ((hi.z + lake.shore_l) / BASKET_L).floor() as i64;
            for i in i0..=i1 {
                for j in j0..=j1 {
                    baskets.entry((i, j)).or_default().push(k);
                }
            }
        }
        let net = Self {
            plate_id,
            lakes,
            baskets,
        };
        net.check()?;
        Ok(net)
    }

    pub const fn plate_id(&self) -> u64 {
        self.plate_id
    }

    pub fn lakes(&self) -> &[LakeSurface] {
        &self.lakes
    }

    pub fn basket_count(&self) -> usize {
        self.baskets.len()
    }

    pub fn kind_count(&self, kind: LakeKind) -> usize {
        self.lakes.iter().filter(|lake| lake.kind == kind).count()
    }

    pub fn water_area_l2(&self) -> f64 {
        self.lakes
            .iter()
            .filter(|lake| lake.kind.has_water())
            .map(|lake| lake.water_area_l2)
            .sum()
    }

    pub fn unresolved_count(&self) -> usize {
        self.kind_count(LakeKind::ClosedSurplus)
    }

    pub fn query(&self, point: Vec2) -> LakeQuery {
        let mut out = LakeQuery {
            networks_considered: 1,
            ..LakeQuery::default()
        };
        let key = (
            (point.x / BASKET_L).floor() as i64,
            (point.z / BASKET_L).floor() as i64,
        );
        let Some(candidates) = self.baskets.get(&key) else {
            return out;
        };
        for &k in &candidates.iter().copied().collect::<BTreeSet<_>>() {
            let lake = &self.lakes[k];
            let mut inside = false;
            let mut distance_l = f64::INFINITY;
            for ring in &lake.rings {
                for (i, &a) in ring.iter().enumerate() {
                    let b = ring[(i + 1) % ring.len()];
                    if (a.z > point.z) != (b.z > point.z)
                        && point.x < (b.x - a.x) * (point.z - a.z) / (b.z - a.z) + a.x
                    {
                        inside = !inside;
                    }
                    distance_l = distance_l.min(distance_to_segment(point, a, b));
                }
            }
            if inside {
                distance_l = 0.0;
            }
            if !inside && distance_l > lake.shore_l {
                continue;
            }
            let shore_strength = if inside {
                1.0
            } else {
                1.0 - smoothstep(0.0, lake.shore_l, distance_l)
            };
            let water_strength = if inside && lake.kind.has_water() {
                1.0
            } else {
                0.0
            };
            out.water = out.water.max(water_strength);
            out.shore = out.shore.max(shore_strength);
            out.basin = out.basin.max(shore_strength);
            let hit = LakeHit {
                plate_id: self.plate_id,
                lake: k,
                depression: lake.depression,
                kind: lake.kind,
                distance_l,
                inside,
                level_km: lake.level_km,
                max_depth_km: lake.max_depth_km,
                frozen: lake.frozen,
                rift: lake.rift,
                high: lake.high,
                water_strength,
                shore_strength,
            };
            if out.nearest.is_none_or(|old| distance_l < old.distance_l) {
                out.nearest = Some(hit);
            }
        }
        out
    }

    fn check(&self) -> Result<(), RiverNetError> {
        for (k, lake) in self.lakes.iter().enumerate() {
            let reason = if lake.rings.is_empty()
                || lake.rings.iter().any(|ring| ring.len() < 3)
                || !lake.center.x.is_finite()
                || !lake.center.z.is_finite()
                || !lake.shore_l.is_finite()
                || lake.shore_l <= 0.0
                || !lake.water_area_l2.is_finite()
                || lake.water_area_l2 < 0.0
                || lake.water_area_l2 > lake.basin_area_l2 + 1e-12
                || lake.level_km.is_some_and(|level| !level.is_finite())
                || (!lake.kind.has_water()
                    && (lake.level_km.is_some() || lake.water_area_l2 != 0.0))
            {
                Some("некорректная поверхность озера")
            } else {
                None
            };
            if let Some(reason) = reason {
                return Err(RiverNetError::BrokenInvariant { node: k, reason });
            }
        }
        Ok(())
    }
}

fn validate(params: &Params) -> Result<(), RiverNetError> {
    let p = &params.lakes;
    if !p.inland_sea_area_l2.is_finite()
        || p.inland_sea_area_l2 <= 0.0
        || !p.swamp_max_depth_km.is_finite()
        || p.swamp_max_depth_km < 0.0
        || !p.swamp_min_p_pet.is_finite()
        || p.swamp_min_p_pet < 1.0
        || !p.frozen_t_min_c.is_finite()
        || !p.shore_fraction.is_finite()
        || !(0.0..=1.0).contains(&p.shore_fraction)
        || p.shore_fraction == 0.0
    {
        return Err(RiverNetError::InvalidParameters(
            "параметры озёр: площадь/глубина неотрицательны, P/PET >= 1, берег в (0,1]",
        ));
    }
    Ok(())
}

/// Контролирующий порог — минимальная седловина среди выходов впадины.
/// `DepressionKind::Filled.spill_km` остаётся диагностикой priority-flood с ε;
/// поверхность озера не должна наследовать этот искусственный наклон.
fn spill_level(
    nodes: &[DrainageNode],
    members: &[usize],
    depression: usize,
    bottom: usize,
) -> Result<f64, RiverNetError> {
    let mut best: Option<(f64, usize)> = None;
    for &k in members {
        let Some(down) = nodes[k].downstream else {
            continue;
        };
        if down >= nodes.len() || nodes[down].depression == Some(depression) {
            continue;
        }
        let saddle = nodes[k].route_km.max(nodes[down].route_km);
        if best.is_none_or(|(level, old)| (saddle, k) < (level, old)) {
            best = Some((saddle, k));
        }
    }
    best.map(|(level, _)| level.max(nodes[bottom].route_km))
        .ok_or(RiverNetError::BrokenInvariant {
            node: members[0],
            reason: "заполненная впадина без выхода для озера",
        })
}

#[derive(Clone, Copy, Debug)]
struct Edge {
    a: Cell,
    b: Cell,
}

/// Граница объединения клеток: внешние кольца и острова. На диагональном
/// касании выбираем правый поворот, чтобы не склеивать клетки через вершину.
fn contours(
    nodes: &[DrainageNode],
    mask: &[bool],
    h: f64,
) -> Result<Vec<Vec<Vec2>>, RiverNetError> {
    let wet: BTreeSet<Cell> = nodes
        .iter()
        .zip(mask)
        .filter(|(_, wet)| **wet)
        .map(|(node, _)| node.cell)
        .collect();
    let mut edges = Vec::new();
    let mut starts = BTreeMap::<Cell, Vec<usize>>::new();
    let mut add = |a: Cell, b: Cell| -> Result<(), RiverNetError> {
        let k = edges.len();
        edges.push(Edge { a, b });
        starts.entry(a).or_default().push(k);
        Ok(())
    };
    for &(i, j) in &wet {
        let corner =
            |di: i64, dj: i64| -> Option<Cell> { Some((i.checked_add(di)?, j.checked_add(dj)?)) };
        let (Some(a), Some(b), Some(c), Some(d)) =
            (corner(0, 0), corner(1, 0), corner(1, 1), corner(0, 1))
        else {
            return Err(RiverNetError::InvalidParameters(
                "клетка озера вышла за диапазон i64",
            ));
        };
        // Interior слева от обхода: внешние кольца положительные, острова отрицательные.
        let has = |di: i64, dj: i64| {
            i.checked_add(di)
                .zip(j.checked_add(dj))
                .is_some_and(|cell| wet.contains(&cell))
        };
        if !has(0, -1) {
            add(a, b)?;
        }
        if !has(1, 0) {
            add(b, c)?;
        }
        if !has(0, 1) {
            add(c, d)?;
        }
        if !has(-1, 0) {
            add(d, a)?;
        }
    }
    let mut used = vec![false; edges.len()];
    let mut rings = Vec::new();
    for first in 0..edges.len() {
        if used[first] {
            continue;
        }
        let start = edges[first].a;
        let mut current = first;
        let mut corners = Vec::new();
        loop {
            if used[current] || corners.len() > edges.len() {
                return Err(RiverNetError::BrokenInvariant {
                    node: 0,
                    reason: "цикл или разрыв в контуре озера",
                });
            }
            used[current] = true;
            let edge = edges[current];
            corners.push(edge.a);
            if edge.b == start {
                break;
            }
            let incoming = (edge.b.0 - edge.a.0, edge.b.1 - edge.a.1);
            let candidates = starts.get(&edge.b).ok_or(RiverNetError::BrokenInvariant {
                node: 0,
                reason: "разрыв в контуре озера",
            })?;
            let mut best: Option<(i64, usize)> = None;
            for &next in candidates {
                if used[next] {
                    continue;
                }
                let out = (edges[next].b.0 - edge.b.0, edges[next].b.1 - edge.b.1);
                let cross = incoming.0 * out.1 - incoming.1 * out.0;
                if best.is_none_or(|(old_cross, old)| (cross, next) < (old_cross, old)) {
                    best = Some((cross, next));
                }
            }
            current = best
                .ok_or(RiverNetError::BrokenInvariant {
                    node: 0,
                    reason: "контур озера не замкнут",
                })?
                .1;
        }
        let ring: Vec<Vec2> = corners
            .iter()
            .map(|&(i, j)| Vec2::new(i as f64 * h, j as f64 * h))
            .collect();
        rings.push(remove_collinear(ring));
    }
    Ok(rings)
}

fn remove_collinear(ring: Vec<Vec2>) -> Vec<Vec2> {
    let n = ring.len();
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let a = ring[(k + n - 1) % n];
        let b = ring[k];
        let c = ring[(k + 1) % n];
        let ab = b.sub(a);
        let bc = c.sub(b);
        if (ab.x * bc.z - ab.z * bc.x).abs() > 1e-12 {
            out.push(b);
        }
    }
    out
}

fn distance_to_segment(point: Vec2, a: Vec2, b: Vec2) -> f64 {
    let ab = b.sub(a);
    let len2 = ab.dot(ab);
    let t = if len2 > 0.0 {
        clamp(point.sub(a).dot(ab) / len2, 0.0, 1.0)
    } else {
        0.0
    };
    point.sub(a.add(ab.scale(t))).len()
}

#[cfg(test)]
mod tests {
    use super::{LakeKind, LakeNetwork, contours};
    use crate::drainage::{Depression, DepressionKind, DrainageNode, DrainageOutlet};
    use crate::math::Vec2;
    use crate::mode::Mode;
    use crate::params::Params;
    use crate::runoff::{ClimateGrid, Runoff, RunoffClimate};
    use crate::version::GenVersion;

    fn params() -> Params {
        Params::defaults(GenVersion::V1Dev, Mode::Gameplay)
    }

    /// Квадратная впадина n×n с дном 0 и порогом `level`; один выход в море.
    /// `fake_spill` намеренно не равен истинному порогу.
    fn basin(
        n: i64,
        level: f64,
        fake_spill: f64,
        closed: bool,
        p_mm: f64,
        pet_mm: f64,
        t_min_c: f64,
    ) -> (Vec<DrainageNode>, Vec<Depression>, Runoff, Params) {
        let p = params();
        let h = p.drainage.step_l;
        let mut nodes = Vec::new();
        for i in 0..n {
            for j in 0..n {
                let bottom = i == n / 2 && j == n / 2;
                nodes.push(DrainageNode {
                    cell: (i, j),
                    point: Vec2::new(i as f64 * h, j as f64 * h),
                    in_plate: true,
                    route_km: if bottom { 0.0 } else { level },
                    filled_km: fake_spill,
                    downstream: None,
                    outlet: DrainageOutlet::None,
                    depression: Some(0),
                });
            }
        }
        let exit = ((n - 1) * n) as usize;
        let sea = nodes.len();
        if !closed {
            nodes[exit].downstream = Some(sea);
            nodes.push(DrainageNode {
                cell: (n, 0),
                point: Vec2::new(n as f64 * h, 0.0),
                in_plate: true,
                route_km: level - 0.2,
                filled_km: level - 0.2,
                downstream: None,
                outlet: DrainageOutlet::Sea,
                depression: None,
            });
        }
        let bottom = ((n / 2) * n + n / 2) as usize;
        let depressions = vec![Depression {
            kind: if closed {
                DepressionKind::Closed
            } else {
                DepressionKind::Filled {
                    spill_km: fake_spill,
                }
            },
            bottom_node: bottom,
            node_count: (n * n) as usize,
            plate_node_count: (n * n) as usize,
            max_depth_km: level.abs(),
        }];
        let points: Vec<_> = nodes.iter().map(|node| node.point).collect();
        let grid = ClimateGrid::build(&points, 1.0, |_| {
            Some(RunoffClimate {
                p_mm,
                pet_mm,
                t_min_c,
            })
        })
        .unwrap();
        let runoff = Runoff::calculate(&nodes, &depressions, &grid, &p).unwrap();
        (nodes, depressions, runoff, p)
    }

    fn build(
        n: i64,
        level: f64,
        fake_spill: f64,
        closed: bool,
        p_mm: f64,
        pet_mm: f64,
        t_min_c: f64,
    ) -> LakeNetwork {
        let (nodes, depressions, runoff, p) =
            basin(n, level, fake_spill, closed, p_mm, pet_mm, t_min_c);
        LakeNetwork::build(7, &nodes, &depressions, &runoff, &p).unwrap()
    }

    #[test]
    fn spill_uses_true_saddle_not_priority_flood_epsilon() {
        let net = build(3, 0.1, 1.0, false, 200.0, 50.0, 10.0);
        let lake = &net.lakes()[0];
        assert_eq!(lake.kind, LakeKind::Lake);
        assert_eq!(lake.level_km, Some(0.1));
        assert!((lake.water_area_l2 - 9.0 * 0.25 * 0.25).abs() < 1e-12);
        assert!((lake.max_depth_km - 0.1).abs() < 1e-12);
        assert_eq!(lake.rings.len(), 1);
        let inside = net.query(Vec2::new(0.25, 0.25));
        assert_eq!(inside.water, 1.0);
        assert!(inside.nearest.unwrap().inside);
        assert_eq!(net.query(Vec2::new(5.0, 5.0)).water, 0.0);
    }

    #[test]
    fn dry_endorheic_basin_has_influence_but_no_water_level() {
        let net = build(3, 0.1, 1.0, false, 10.0, 100.0, 10.0);
        let lake = &net.lakes()[0];
        assert_eq!(lake.kind, LakeKind::DryBasin);
        assert_eq!(lake.level_km, None);
        assert_eq!(lake.water_area_l2, 0.0);
        let query = net.query(Vec2::new(0.25, 0.25));
        assert_eq!(query.water, 0.0);
        assert_eq!(query.basin, 1.0);
    }

    #[test]
    fn shallow_humid_basin_is_swamp_and_large_basin_is_inland_sea() {
        let swamp = build(3, 0.005, 1.0, false, 200.0, 50.0, 10.0);
        assert_eq!(swamp.lakes()[0].kind, LakeKind::Swamp);
        let sea = build(5, 0.1, 1.0, false, 200.0, 50.0, 10.0);
        assert_eq!(sea.lakes()[0].kind, LakeKind::InlandSea);
        assert!(sea.lakes()[0].water_area_l2 > 1.0);
    }

    #[test]
    fn closed_surplus_does_not_invent_a_spill_or_water_level() {
        let net = build(3, 0.1, 1.0, true, 200.0, 50.0, 10.0);
        assert_eq!(net.lakes()[0].kind, LakeKind::ClosedSurplus);
        assert_eq!(net.lakes()[0].level_km, None);
        assert_eq!(net.unresolved_count(), 1);
        assert_eq!(net.query(Vec2::new(0.25, 0.25)).water, 0.0);
    }

    #[test]
    fn freezing_winter_marks_the_lake_and_high_water_stays_r2() {
        let net = build(3, 0.1, 1.0, false, 200.0, 50.0, -5.0);
        let lake = &net.lakes()[0];
        assert!(lake.frozen);
        assert!(lake.high);
        assert_eq!(lake.level_km, Some(0.1));
    }

    #[test]
    fn negative_spill_is_sea_connected_not_a_freshwater_lake() {
        let net = build(3, -0.1, -0.05, false, 200.0, 50.0, 10.0);
        let lake = &net.lakes()[0];
        assert_eq!(lake.kind, LakeKind::SeaConnected);
        assert_eq!(lake.level_km, Some(0.0));
        assert!(!lake.high);
    }

    #[test]
    fn contours_keep_islands_as_holes() {
        let h = 0.25;
        let mut nodes = Vec::new();
        let mut mask = Vec::new();
        for i in 0..3 {
            for j in 0..3 {
                nodes.push(DrainageNode {
                    cell: (i, j),
                    point: Vec2::new(i as f64 * h, j as f64 * h),
                    in_plate: true,
                    route_km: 0.0,
                    filled_km: 0.0,
                    downstream: None,
                    outlet: DrainageOutlet::None,
                    depression: None,
                });
                mask.push(!(i == 1 && j == 1));
            }
        }
        let rings = contours(&nodes, &mask, h).unwrap();
        assert_eq!(rings.len(), 2);
        let area: f64 = rings
            .iter()
            .map(|ring| {
                let mut sum = 0.0;
                for (k, &a) in ring.iter().enumerate() {
                    let b = ring[(k + 1) % ring.len()];
                    sum += a.x * b.z - b.x * a.z;
                }
                sum * 0.5
            })
            .sum();
        assert!((area - 8.0 * h * h).abs() < 1e-12);
    }
}
