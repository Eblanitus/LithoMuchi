//! Геометрия русел поверх расхода T111 (PLAN-REVISION 5.4, T112).
//!
//! Цепочки узлов превращаются в сглаженные полилинии, ширина растёт как √Q,
//! меандры смещают полилинию по нормали, отрезки раскладываются в корзины
//! 0.5 L. Это пока диагностика: `field()` и ванильные C/E/W/PV не меняются.

use std::collections::{BTreeSet, HashMap};

use crate::drainage::{DrainageNode, RiverNetError};
use crate::hash::{hash3, unit};
use crate::math::{Vec2, clamp, dpow, lerp, smoothstep};
use crate::noise::noise2;
use crate::params::Params;
use crate::runoff::{FlowTerminal, Runoff};
use crate::seed::{SALT_CHANNEL_MEANDER, subseed};

type Cell = (i64, i64);

/// Инфраструктурный размер корзины. На ответ не влияет: сегмент кладётся во
/// все корзины, которых касается его AABB, расширенный на долину.
pub const BASKET_L: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReachEnd {
    Sea,
    Rift,
    Transform,
    Lake,
    Endorheic,
    Confluence,
    /// Следующий узел ниже Q_min; не выдаётся за полноценное устье.
    BelowThreshold,
    /// Кольцо соседней плиты; связь добавит T113.
    Boundary,
    /// Замкнутый избыток T111; контур/выход уточнят T113/T114.
    ClosedSurplus,
}

impl ReachEnd {
    pub const fn is_unresolved(self) -> bool {
        matches!(
            self,
            Self::BelowThreshold | Self::Boundary | Self::ClosedSurplus
        )
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sea => "sea",
            Self::Rift => "rift",
            Self::Transform => "transform",
            Self::Lake => "lake",
            Self::Endorheic => "endorheic",
            Self::Confluence => "confluence",
            Self::BelowThreshold => "below_threshold",
            Self::Boundary => "boundary",
            Self::ClosedSurplus => "closed_surplus",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelPoint {
    pub point: Vec2,
    pub discharge_mm_l2: f64,
    pub core_l: f64,
    pub valley_l: f64,
    pub depth_blocks: f64,
    pub meander_amplitude_l: f64,
    pub meander_lambda_l: f64,
    pub slope_km_l: f64,
}

impl ChannelPoint {
    fn lerp(self, other: Self, t: f64) -> Self {
        Self {
            point: Vec2::new(
                lerp(self.point.x, other.point.x, t),
                lerp(self.point.z, other.point.z, t),
            ),
            discharge_mm_l2: lerp(self.discharge_mm_l2, other.discharge_mm_l2, t),
            core_l: lerp(self.core_l, other.core_l, t),
            valley_l: lerp(self.valley_l, other.valley_l, t),
            depth_blocks: lerp(self.depth_blocks, other.depth_blocks, t),
            meander_amplitude_l: lerp(self.meander_amplitude_l, other.meander_amplitude_l, t),
            meander_lambda_l: lerp(self.meander_lambda_l, other.meander_lambda_l, t),
            slope_km_l: lerp(self.slope_km_l, other.slope_km_l, t),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChannelReach {
    pub source_node: usize,
    pub mouth_node: usize,
    pub end: ReachEnd,
    pub length_l: f64,
    pub sinuosity: f64,
    pub plain: bool,
    first_segment: usize,
    segment_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelSegment {
    pub a: ChannelPoint,
    pub b: ChannelPoint,
    pub reach: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelHit {
    pub segment: usize,
    pub reach: usize,
    pub distance_l: f64,
    pub discharge_mm_l2: f64,
    pub core_l: f64,
    pub valley_l: f64,
    pub depth_blocks: f64,
    pub core_strength: f64,
    pub valley_strength: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChannelQuery {
    pub core: f64,
    pub valley: f64,
    pub nearest: Option<ChannelHit>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChannelNetwork {
    reaches: Vec<ChannelReach>,
    segments: Vec<ChannelSegment>,
    baskets: HashMap<Cell, Vec<usize>>,
    max_valley_l: f64,
}

impl ChannelNetwork {
    pub fn build(
        plate_id: u64,
        seed: u64,
        nodes: &[DrainageNode],
        runoff: &Runoff,
        params: &Params,
    ) -> Result<Self, RiverNetError> {
        validate(params)?;
        if runoff.nodes().len() != nodes.len() {
            return Err(RiverNetError::BrokenInvariant {
                node: 0,
                reason: "расход и топография имеют разную длину",
            });
        }
        let q_min = params.channels.q_min_mm_l2;
        let active: Vec<bool> = nodes
            .iter()
            .enumerate()
            .map(|(k, node)| {
                node.in_plate
                    && node.depression.is_none()
                    && node.route_km >= 0.0
                    && runoff.nodes()[k].discharge_mm_l2 >= q_min
            })
            .collect();
        let mut upstream = vec![0_usize; nodes.len()];
        for (k, flow) in runoff.nodes().iter().enumerate() {
            if active[k] {
                if let Some(down) = flow.downstream {
                    if down < nodes.len() && active[down] {
                        upstream[down] += 1;
                    }
                }
            }
        }
        let mut segments = Vec::new();
        let mut reaches = Vec::new();
        let meander_seed = subseed(seed, SALT_CHANNEL_MEANDER);
        for head in 0..nodes.len() {
            if !active[head] || upstream[head] == 1 {
                continue;
            }
            let mut chain = vec![head];
            let mut current = head;
            let end = loop {
                let flow = &runoff.nodes()[current];
                let Some(down) = flow.downstream else {
                    break terminal_end(flow.terminal);
                };
                if down >= nodes.len() {
                    return Err(RiverNetError::BrokenInvariant {
                        node: current,
                        reason: "индекс русла вне сети",
                    });
                }
                if !active[down] {
                    chain.push(down);
                    break if nodes[down].depression.is_some() {
                        terminal_end(runoff.nodes()[down].terminal)
                    } else if !nodes[down].in_plate {
                        ReachEnd::Boundary
                    } else if nodes[down].route_km < 0.0 {
                        ReachEnd::Sea
                    } else {
                        ReachEnd::BelowThreshold
                    };
                }
                chain.push(down);
                if upstream[down] != 1 {
                    break ReachEnd::Confluence;
                }
                if chain.len() > nodes.len() {
                    return Err(RiverNetError::BrokenInvariant {
                        node: head,
                        reason: "цикл в русле",
                    });
                }
                current = down;
            };
            if chain.len() < 2 {
                continue;
            }
            let base = chain_vertices(&chain, nodes, runoff, params);
            let smoothed = chaikin(&chaikin(&base));
            let sampled = resample(&smoothed, params.drainage.step_l);
            let meandered = meander(&sampled, meander_seed, plate_id, reaches.len());
            let first_segment = segments.len();
            for pair in meandered.windows(2) {
                if pair[0].point.dist2(pair[1].point) <= 1e-24 {
                    continue;
                }
                segments.push(ChannelSegment {
                    a: pair[0],
                    b: pair[1],
                    reach: reaches.len(),
                });
            }
            let segment_count = segments.len() - first_segment;
            if segment_count == 0 {
                continue;
            }
            let length_l: f64 = segments[first_segment..]
                .iter()
                .map(|s| s.a.point.sub(s.b.point).len())
                .sum();
            let chord = meandered
                .first()
                .expect("полилиния непуста")
                .point
                .sub(meandered.last().expect("полилиния непуста").point)
                .len();
            let mean_slope = base.iter().map(|v| v.slope_km_l).sum::<f64>() / base.len() as f64;
            reaches.push(ChannelReach {
                source_node: head,
                mouth_node: *chain.last().expect("цепочка непуста"),
                end,
                length_l,
                sinuosity: if chord > 1e-9 { length_l / chord } else { 0.0 },
                plain: mean_slope < params.channels.plain_slope_max,
                first_segment,
                segment_count,
            });
        }
        let mut baskets = HashMap::<Cell, Vec<usize>>::new();
        let mut max_valley_l = 0.0_f64;
        for (k, segment) in segments.iter().enumerate() {
            let radius = segment.a.valley_l.max(segment.b.valley_l);
            max_valley_l = max_valley_l.max(radius);
            let min_x = segment.a.point.x.min(segment.b.point.x) - radius;
            let max_x = segment.a.point.x.max(segment.b.point.x) + radius;
            let min_z = segment.a.point.z.min(segment.b.point.z) - radius;
            let max_z = segment.a.point.z.max(segment.b.point.z) + radius;
            let i0 = (min_x / BASKET_L).floor() as i64;
            let i1 = (max_x / BASKET_L).floor() as i64;
            let j0 = (min_z / BASKET_L).floor() as i64;
            let j1 = (max_z / BASKET_L).floor() as i64;
            for i in i0..=i1 {
                for j in j0..=j1 {
                    baskets.entry((i, j)).or_default().push(k);
                }
            }
        }
        let net = Self {
            reaches,
            segments,
            baskets,
            max_valley_l,
        };
        net.check()?;
        Ok(net)
    }

    pub fn reaches(&self) -> &[ChannelReach] {
        &self.reaches
    }

    pub fn segments(&self) -> &[ChannelSegment] {
        &self.segments
    }

    pub fn basket_count(&self) -> usize {
        self.baskets.len()
    }

    pub const fn max_valley_l(&self) -> f64 {
        self.max_valley_l
    }

    pub fn total_length_l(&self) -> f64 {
        self.reaches.iter().map(|r| r.length_l).sum()
    }

    pub fn unresolved_count(&self) -> usize {
        self.reaches
            .iter()
            .filter(|r| r.end.is_unresolved())
            .count()
    }

    pub fn end_count(&self, end: ReachEnd) -> usize {
        self.reaches.iter().filter(|r| r.end == end).count()
    }

    /// Быстрый запрос одной плиты: только корзина точки, потому что сегменты
    /// заранее разложены по AABB с запасом на долину. Межплитовый поиск — T113.
    pub fn query(&self, point: Vec2) -> ChannelQuery {
        let key = (
            (point.x / BASKET_L).floor() as i64,
            (point.z / BASKET_L).floor() as i64,
        );
        let Some(candidates) = self.baskets.get(&key) else {
            return ChannelQuery::default();
        };
        let unique: BTreeSet<_> = candidates.iter().copied().collect();
        let mut out = ChannelQuery::default();
        for k in unique {
            let segment = self.segments[k];
            let ab = segment.b.point.sub(segment.a.point);
            let len2 = ab.dot(ab);
            let t = if len2 > 0.0 {
                clamp(point.sub(segment.a.point).dot(ab) / len2, 0.0, 1.0)
            } else {
                0.0
            };
            let at = segment.a.lerp(segment.b, t);
            let distance_l = point.sub(at.point).len();
            if distance_l > at.valley_l {
                continue;
            }
            let core_strength = 1.0 - smoothstep(at.core_l, 1.5 * at.core_l, distance_l);
            let valley_strength = 1.0 - smoothstep(0.0, at.valley_l, distance_l);
            out.core = out.core.max(core_strength);
            out.valley = out.valley.max(valley_strength);
            let hit = ChannelHit {
                segment: k,
                reach: segment.reach,
                distance_l,
                discharge_mm_l2: at.discharge_mm_l2,
                core_l: at.core_l,
                valley_l: at.valley_l,
                depth_blocks: at.depth_blocks,
                core_strength,
                valley_strength,
            };
            if out.nearest.is_none_or(|old| distance_l < old.distance_l) {
                out.nearest = Some(hit);
            }
        }
        out
    }

    fn check(&self) -> Result<(), RiverNetError> {
        for (k, segment) in self.segments.iter().enumerate() {
            if segment.reach >= self.reaches.len()
                || !segment.a.point.x.is_finite()
                || !segment.a.point.z.is_finite()
                || !segment.b.point.x.is_finite()
                || !segment.b.point.z.is_finite()
                || segment.a.core_l <= 0.0
                || segment.b.core_l <= 0.0
                || segment.a.valley_l < segment.a.core_l
                || segment.b.valley_l < segment.b.core_l
            {
                return Err(RiverNetError::BrokenInvariant {
                    node: k,
                    reason: "некорректный сегмент русла",
                });
            }
        }
        Ok(())
    }
}

fn validate(params: &Params) -> Result<(), RiverNetError> {
    let c = &params.channels;
    let values = [
        c.q_min_mm_l2,
        c.core_min_blocks,
        c.core_max_blocks,
        c.depth0_blocks,
        c.meander_plain,
        c.meander_mountain,
        c.valley_plain,
        c.valley_mountain,
        c.plain_slope_max,
    ];
    if values.iter().any(|v| !v.is_finite() || *v <= 0.0)
        || c.core_max_blocks < c.core_min_blocks
        || c.valley_plain < 1.0
        || c.valley_mountain < 1.0
    {
        return Err(RiverNetError::InvalidParameters(
            "параметры русла должны быть положительными, core_max >= core_min, valley >= 1",
        ));
    }
    Ok(())
}

fn terminal_end(terminal: FlowTerminal) -> ReachEnd {
    match terminal {
        FlowTerminal::Sea => ReachEnd::Sea,
        FlowTerminal::Rift => ReachEnd::Rift,
        FlowTerminal::Transform => ReachEnd::Transform,
        FlowTerminal::Lake { .. } => ReachEnd::Lake,
        FlowTerminal::Endorheic { .. } => ReachEnd::Endorheic,
        FlowTerminal::ClosedSurplus { .. } => ReachEnd::ClosedSurplus,
        FlowTerminal::Boundary => ReachEnd::Boundary,
        FlowTerminal::Flowing => ReachEnd::BelowThreshold,
    }
}

fn slope_at(nodes: &[DrainageNode], k: usize) -> f64 {
    let Some(down) = nodes[k].downstream else {
        return 0.0;
    };
    if down >= nodes.len() {
        return 0.0;
    }
    let distance = nodes[k].point.sub(nodes[down].point).len();
    if distance <= 1e-9 {
        return 0.0;
    }
    ((nodes[k].filled_km - nodes[down].filled_km) / distance).max(0.0)
}

fn vertex(point: Vec2, q: f64, slope: f64, params: &Params) -> ChannelPoint {
    let c = &params.channels;
    let l = params.block.l_blocks;
    let ratio = (q / c.q_min_mm_l2).max(1.0);
    let core_blocks = clamp(
        c.core_min_blocks * dpow(ratio, 0.5),
        c.core_min_blocks,
        c.core_max_blocks,
    );
    let plain_weight = 1.0 - smoothstep(0.5 * c.plain_slope_max, c.plain_slope_max, slope);
    let meander_k = lerp(c.meander_mountain, c.meander_plain, plain_weight);
    let valley_k = lerp(c.valley_mountain, c.valley_plain, plain_weight);
    ChannelPoint {
        point,
        discharge_mm_l2: q,
        core_l: core_blocks / l,
        valley_l: valley_k * core_blocks / l,
        depth_blocks: c.depth0_blocks * dpow(ratio, 0.4),
        meander_amplitude_l: meander_k * 2.0 * core_blocks / l,
        meander_lambda_l: 11.0 * 2.0 * core_blocks / l,
        slope_km_l: slope,
    }
}

fn chain_vertices(
    chain: &[usize],
    nodes: &[DrainageNode],
    runoff: &Runoff,
    params: &Params,
) -> Vec<ChannelPoint> {
    let mut out = Vec::with_capacity(chain.len());
    for (position, &k) in chain.iter().enumerate() {
        let q = runoff.nodes()[k].discharge_mm_l2;
        let slope = slope_at(nodes, k);
        // Конечный узел может быть морем/кольцом с другим Q; ширину берём
        // по последнему речному узлу, чтобы устье не схлопнулось в ноль.
        let (q, slope) = if position + 1 == chain.len() && position > 0 {
            let prev = chain[position - 1];
            (runoff.nodes()[prev].discharge_mm_l2, slope_at(nodes, prev))
        } else {
            (q, slope)
        };
        out.push(vertex(nodes[k].point, q, slope, params));
    }
    out
}

/// Открытая схема Чайкина: концы сохраняются, углы сетки сглаживаются.
fn chaikin(input: &[ChannelPoint]) -> Vec<ChannelPoint> {
    if input.len() < 3 {
        return input.to_vec();
    }
    let mut out = Vec::with_capacity(input.len() * 2);
    out.push(input[0]);
    for pair in input.windows(2) {
        out.push(pair[0].lerp(pair[1], 0.25));
        out.push(pair[0].lerp(pair[1], 0.75));
    }
    out.push(*input.last().expect("полилиния непуста"));
    out
}

fn resample(input: &[ChannelPoint], node_step_l: f64) -> Vec<ChannelPoint> {
    if input.len() < 2 {
        return input.to_vec();
    }
    let mut out = vec![input[0]];
    for pair in input.windows(2) {
        let distance = pair[0].point.sub(pair[1].point).len();
        if distance <= 1e-12 {
            continue;
        }
        let lambda = pair[0].meander_lambda_l.min(pair[1].meander_lambda_l);
        let step = clamp(lambda / 8.0, node_step_l / 8.0, node_step_l / 2.0);
        let count = (distance / step).ceil().max(1.0) as usize;
        for k in 1..=count {
            out.push(pair[0].lerp(pair[1], k as f64 / count as f64));
        }
    }
    out
}

fn meander(input: &[ChannelPoint], seed: u64, plate_id: u64, reach: usize) -> Vec<ChannelPoint> {
    if input.len() < 3 {
        return input.to_vec();
    }
    let total: f64 = input
        .windows(2)
        .map(|pair| pair[0].point.sub(pair[1].point).len())
        .sum();
    if total <= 1e-9 {
        return input.to_vec();
    }
    let end_ramp = (0.5_f64).min(total * 0.25);
    let offset = unit(hash3(seed, plate_id as i64, reach as i64, 7)) * 1024.0;
    let mut out = Vec::with_capacity(input.len());
    let mut distance = 0.0;
    let mut phase = 0.0;
    for k in 0..input.len() {
        if k > 0 {
            let ds = input[k - 1].point.sub(input[k].point).len();
            let lambda = input[k - 1]
                .meander_lambda_l
                .max(input[k].meander_lambda_l)
                .max(1e-9);
            distance += ds;
            phase += ds / lambda;
        }
        let tangent = if k == 0 {
            input[1].point.sub(input[0].point)
        } else if k + 1 == input.len() {
            input[k].point.sub(input[k - 1].point)
        } else {
            input[k + 1].point.sub(input[k - 1].point)
        }
        .normalized();
        let envelope =
            smoothstep(0.0, end_ramp, distance) * smoothstep(total, total - end_ramp, distance);
        let noise = noise2(seed, phase, offset);
        let shift = tangent
            .perp()
            .scale(input[k].meander_amplitude_l * noise * envelope);
        let mut moved = input[k];
        moved.point = input[k].point.add(shift);
        if k == 0 {
            moved.point = input[0].point;
        } else if k + 1 == input.len() {
            moved.point = input[k].point;
        }
        out.push(moved);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{BASKET_L, ChannelNetwork, ReachEnd};
    use crate::drainage::{DrainageNode, DrainageOutlet};
    use crate::math::Vec2;
    use crate::mode::Mode;
    use crate::params::Params;
    use crate::runoff::{ClimateGrid, Runoff, RunoffClimate};
    use crate::version::GenVersion;

    fn params() -> Params {
        Params::defaults(GenVersion::V1Dev, Mode::Gameplay)
    }

    fn node(i: i64, height: f64, down: Option<usize>) -> DrainageNode {
        DrainageNode {
            cell: (i, 0),
            point: Vec2::new(i as f64 * 0.25, 0.0),
            in_plate: true,
            route_km: height,
            filled_km: height,
            downstream: down,
            outlet: if down.is_none() {
                DrainageOutlet::Sea
            } else {
                DrainageOutlet::None
            },
            depression: None,
        }
    }

    fn runoff(nodes: &[DrainageNode], p: &Params, q_scale: f64) -> Runoff {
        let points: Vec<_> = nodes.iter().map(|n| n.point).collect();
        let grid = ClimateGrid::build(&points, 1.0, |_| {
            Some(RunoffClimate {
                p_mm: 100.0 * q_scale,
                pet_mm: 0.0,
            })
        })
        .unwrap();
        Runoff::calculate(nodes, &[], &grid, p).unwrap()
    }

    #[test]
    fn width_grows_downstream_and_basket_query_matches_brute_force() {
        let p = params();
        let nodes = vec![
            node(4, 4.0, Some(1)),
            node(3, 3.0, Some(2)),
            node(2, 2.0, Some(3)),
            node(1, 1.0, None),
        ];
        let flow = runoff(&nodes, &p, 4.0);
        let net = ChannelNetwork::build(7, 42, &nodes, &flow, &p).unwrap();
        assert_eq!(net.reaches().len(), 1);
        assert_eq!(net.reaches()[0].end, ReachEnd::Sea);
        let reach = &net.reaches()[0];
        let first = net.segments()[reach.first_segment].a;
        let last = net.segments()[reach.first_segment + reach.segment_count - 1].b;
        assert!(last.core_l >= first.core_l);
        assert!(net.total_length_l() > 0.0);
        for k in 0..20 {
            let point = Vec2::new(k as f64 * 0.07, 0.02);
            let fast = net.query(point);
            let mut brute = super::ChannelQuery::default();
            for (s, segment) in net.segments().iter().enumerate() {
                let ab = segment.b.point.sub(segment.a.point);
                let t =
                    crate::math::clamp(point.sub(segment.a.point).dot(ab) / ab.dot(ab), 0.0, 1.0);
                let at = segment.a.lerp(segment.b, t);
                let d = point.sub(at.point).len();
                if d <= at.valley_l {
                    let core = 1.0 - crate::math::smoothstep(at.core_l, 1.5 * at.core_l, d);
                    let valley = 1.0 - crate::math::smoothstep(0.0, at.valley_l, d);
                    brute.core = brute.core.max(core);
                    brute.valley = brute.valley.max(valley);
                    if brute
                        .nearest
                        .is_none_or(|old: super::ChannelHit| d < old.distance_l)
                    {
                        brute.nearest = Some(super::ChannelHit {
                            segment: s,
                            reach: segment.reach,
                            distance_l: d,
                            discharge_mm_l2: at.discharge_mm_l2,
                            core_l: at.core_l,
                            valley_l: at.valley_l,
                            depth_blocks: at.depth_blocks,
                            core_strength: core,
                            valley_strength: valley,
                        });
                    }
                }
            }
            assert_eq!(fast, brute, "point={point:?} basket={BASKET_L}");
        }
    }

    #[test]
    fn dry_network_has_no_channels_and_meanders_are_deterministic() {
        let p = params();
        let nodes = vec![node(1, 1.0, Some(1)), node(0, 0.0, None)];
        let dry = runoff(&nodes, &p, 0.01);
        let empty = ChannelNetwork::build(7, 42, &nodes, &dry, &p).unwrap();
        assert!(empty.reaches().is_empty());
        let wet = runoff(&nodes, &p, 4.0);
        let a = ChannelNetwork::build(7, 42, &nodes, &wet, &p).unwrap();
        let b = ChannelNetwork::build(7, 42, &nodes, &wet, &p).unwrap();
        assert_eq!(a, b);
    }
}
