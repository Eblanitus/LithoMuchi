//! Геометрия русел поверх расхода T111 (PLAN-REVISION 5.4, T112/T114).
//!
//! Цепочки узлов превращаются в сглаженные полилинии, ширина растёт как √Q,
//! меандры смещают полилинию по нормали, отрезки раскладываются в корзины
//! 0.5 L. Общий запрос соседних плит T112/T113 живёт в генераторе; это пока
//! диагностика: `field()` и ванильные C/E/W/PV от сети не меняются.

use std::collections::{BTreeSet, HashMap};

use crate::drainage::{DrainageNode, RiverNetError};
use crate::hash::{hash3, unit};
use crate::math::{SQRT_2, Vec2, clamp, dpow, lerp, smoothstep};
use crate::noise::noise2;
use crate::params::Params;
use crate::runoff::{FlowTerminal, Runoff};
use crate::seed::{SALT_CHANNEL_DELTA, SALT_CHANNEL_MEANDER, subseed};

type Cell = (i64, i64);

/// Инфраструктурный размер корзины. На ответ не влияет: сегмент кладётся во
/// все корзины, которых касается его AABB, расширенный на долину.
pub const BASKET_L: f64 = 0.5;
/// tan(25°): веер дельты задаётся наклоном к перпендикуляру, без системного
/// sin/cos. Угол рукава относительнооси русла не превышает 25° (§5.4).
const DELTA_TAN_HALF_ANGLE: f64 = 0.466_307_658_154_998_6;

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
    /// Кольцо соседней плиты без выхода в этой сети. T113 накладывает геометрию
    /// соседей общим запросом, но не переносит Q между плитами (§5.5). С Д-058
    /// русло продолжается по узлам кольца, так что этот конец остаётся только
    /// там, где сток кольца вообще не нашёл выхода.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReachKind {
    Channel,
    Delta,
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
    /// T145: высота стока узла, км; источник зеркала русла.
    pub route_km: f64,
    /// T145: зеркало воды в блоках, округлённое и монотонное вниз по течению.
    pub mirror_y: f64,
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
            route_km: lerp(self.route_km, other.route_km, t),
            mirror_y: lerp(self.mirror_y, other.mirror_y, t),
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
    pub kind: ReachKind,
    pub parent_reach: Option<usize>,
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
    pub plate_id: u64,
    pub segment: usize,
    pub reach: usize,
    pub distance_l: f64,
    pub discharge_mm_l2: f64,
    pub core_l: f64,
    pub valley_l: f64,
    pub depth_blocks: f64,
    pub core_strength: f64,
    pub valley_strength: f64,
    pub delta_strength: f64,
    /// T115: тип устья и расстояние вдоль русла до него, не по прямой.
    pub end: ReachEnd,
    pub mouth_distance_l: f64,
    /// T145: зеркало русла в точке запроса, блоки.
    pub mirror_y: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChannelQuery {
    pub core: f64,
    /// Д-057: узкий берег самого русла (вырез высоты и вода). `core` с
    /// широким переходом остаётся для C/E/W и биомов.
    pub bank: f64,
    pub valley: f64,
    pub delta: f64,
    /// T116: максимум силы коридора большой реки, без климатического веса.
    pub gallery: f64,
    pub nearest: Option<ChannelHit>,
    /// Диагностика T113: сколько сетей обошёл общий запрос.
    pub networks_considered: usize,
}

impl ChannelQuery {
    /// Объединение независимых сетей: максимум силы и ближайшее русло.
    /// Ничьи по расстоянию разрешаются id плиты, затем номерами русла/отрезка.
    pub fn merge(&mut self, other: Self) {
        self.core = self.core.max(other.core);
        self.bank = self.bank.max(other.bank);
        self.valley = self.valley.max(other.valley);
        self.delta = self.delta.max(other.delta);
        self.gallery = self.gallery.max(other.gallery);
        self.networks_considered += other.networks_considered;
        if let Some(hit) = other.nearest {
            if self.nearest.is_none_or(|old| {
                hit.distance_l.total_cmp(&old.distance_l).then_with(|| {
                    (hit.plate_id, hit.reach, hit.segment).cmp(&(
                        old.plate_id,
                        old.reach,
                        old.segment,
                    ))
                }) == std::cmp::Ordering::Less
            }) {
                self.nearest = Some(hit);
            }
        }
    }
}

/// Верхняя граница радиуса общего запроса по параметрам, ещё до построения
/// сетей: долина + меандр + переход к кольцевому узлу устья. Последний член —
/// инфраструктурный запас решётки, не новый параметр мира.
pub fn max_query_radius_l(params: &Params) -> f64 {
    let c = &params.channels;
    let core_l = c.core_max_blocks / params.block.l_blocks;
    let reach_factor = (c.valley_plain.max(c.valley_mountain)
        + 2.0 * c.meander_plain.max(c.meander_mountain))
    .max(params.hydrology.gallery_width_factor);
    core_l * reach_factor
        + params.drainage.step_l * SQRT_2
        + c.delta_length_max_l
        + params.hydrology.channel_fade_l
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChannelNetwork {
    plate_id: u64,
    reaches: Vec<ChannelReach>,
    segments: Vec<ChannelSegment>,
    baskets: HashMap<Cell, Vec<usize>>,
    max_valley_l: f64,
    bank_fraction: f64,
    field_fade_l: f64,
    bank_min_l: f64,
    gallery_q_min_mm_l2: f64,
    gallery_width_factor: f64,
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
        crate::hydrology::validate(params)?;
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
                // Д-058: узлы кольца соседней плиты тоже несут русло. Раньше
                // река, ушедшая за границу плиты, обрывалась на первом узле
                // кольца, хотя сток идёт по кольцу дальше до моря, рифта или
                // обратно в плиту. Общий запрос T113 рисует сети всех плит.
                node.depression.is_none()
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
                    } else if nodes[down].outlet.is_outlet() {
                        match nodes[down].outlet {
                            crate::drainage::DrainageOutlet::Sea => ReachEnd::Sea,
                            crate::drainage::DrainageOutlet::Rift => ReachEnd::Rift,
                            crate::drainage::DrainageOutlet::Transform => ReachEnd::Transform,
                            crate::drainage::DrainageOutlet::None => ReachEnd::Boundary,
                        }
                    } else if !nodes[down].in_plate {
                        ReachEnd::Boundary
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
            let mean_slope = base.iter().map(|v| v.slope_km_l).sum::<f64>() / base.len() as f64;
            let plain = mean_slope < params.channels.plain_slope_max;
            let smoothed = chaikin(&chaikin(&base));
            let sampled = resample(&smoothed, params.drainage.step_l);
            let mut meandered = meander(&sampled, meander_seed, plate_id, reaches.len());
            assign_mirror(&mut meandered, plain, params);
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
            reaches.push(ChannelReach {
                source_node: head,
                mouth_node: *chain.last().expect("цепочка непуста"),
                end,
                length_l,
                sinuosity: if chord > 1e-9 { length_l / chord } else { 0.0 },
                plain,
                kind: ReachKind::Channel,
                parent_reach: None,
                first_segment,
                segment_count,
            });
        }
        let (reaches, segments) = append_deltas(reaches, segments, seed, plate_id, params);
        let mut baskets = HashMap::<Cell, Vec<usize>>::new();
        let mut max_valley_l = 0.0_f64;
        let field_fade_l = params.hydrology.channel_fade_l;
        for (k, segment) in segments.iter().enumerate() {
            let end_influence = |p: ChannelPoint| {
                p.valley_l
                    .max(p.core_l + (params.hydrology.bank_fraction * p.core_l).max(field_fade_l))
            };
            let gallery_radius = |p: ChannelPoint| p.core_l * params.hydrology.gallery_width_factor;
            let radius = end_influence(segment.a)
                .max(end_influence(segment.b))
                .max(gallery_radius(segment.a))
                .max(gallery_radius(segment.b));
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
            plate_id,
            reaches,
            segments,
            baskets,
            max_valley_l,
            bank_fraction: params.hydrology.bank_fraction,
            field_fade_l: params.hydrology.channel_fade_l,
            bank_min_l: params.hydrology.bank_min_blocks / params.block.l_blocks,
            gallery_q_min_mm_l2: params.hydrology.gallery_q_min_mm_l2,
            gallery_width_factor: params.hydrology.gallery_width_factor,
        };
        net.check()?;
        Ok(net)
    }

    pub const fn plate_id(&self) -> u64 {
        self.plate_id
    }

    pub fn reaches(&self) -> &[ChannelReach] {
        &self.reaches
    }

    pub fn segments(&self) -> &[ChannelSegment] {
        &self.segments
    }

    pub fn reach_first_segment(&self, reach: usize) -> usize {
        self.reaches[reach].first_segment
    }

    pub fn reach_segments(&self, reach: usize) -> &[ChannelSegment] {
        let reach = &self.reaches[reach];
        &self.segments[reach.first_segment..reach.first_segment + reach.segment_count]
    }

    pub fn basket_count(&self) -> usize {
        self.baskets.len()
    }

    pub const fn max_valley_l(&self) -> f64 {
        self.max_valley_l
    }

    pub fn delta_count(&self) -> usize {
        self.reaches
            .iter()
            .filter_map(|r| (r.kind == ReachKind::Delta).then_some(r.parent_reach))
            .collect::<BTreeSet<_>>()
            .len()
    }

    pub fn delta_arm_count(&self) -> usize {
        self.reaches
            .iter()
            .filter(|r| r.kind == ReachKind::Delta)
            .count()
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
    /// заранее разложены по AABB с запасом на долину. Общий поиск T113 живёт
    /// в `Generator::drainage_channel_query` и объединяет такие запросы.
    pub fn query(&self, point: Vec2) -> ChannelQuery {
        let key = (
            (point.x / BASKET_L).floor() as i64,
            (point.z / BASKET_L).floor() as i64,
        );
        let Some(candidates) = self.baskets.get(&key) else {
            return ChannelQuery {
                networks_considered: 1,
                ..ChannelQuery::default()
            };
        };
        let unique: BTreeSet<_> = candidates.iter().copied().collect();
        let mut out = ChannelQuery {
            networks_considered: 1,
            ..ChannelQuery::default()
        };
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
            let fade = (self.bank_fraction * at.core_l).max(self.field_fade_l);
            let gallery_l = at.core_l * self.gallery_width_factor;
            let influence_l = at.valley_l.max(at.core_l + fade).max(gallery_l);
            if distance_l > influence_l {
                continue;
            }
            let core_strength = 1.0 - smoothstep(at.core_l, at.core_l + fade, distance_l);
            let valley_strength = 1.0 - smoothstep(0.0, influence_l, distance_l);
            let bank_l = (self.bank_fraction * at.core_l).max(self.bank_min_l);
            let bank_strength = 1.0 - smoothstep(at.core_l, at.core_l + bank_l, distance_l);
            out.core = out.core.max(core_strength);
            out.bank = out.bank.max(bank_strength);
            out.valley = out.valley.max(valley_strength);
            let large = smoothstep(
                0.5 * self.gallery_q_min_mm_l2,
                self.gallery_q_min_mm_l2,
                at.discharge_mm_l2,
            );
            out.gallery = out
                .gallery
                .max(large * (1.0 - smoothstep(0.0, gallery_l, distance_l)));
            let delta_strength = if self.reaches[segment.reach].kind == ReachKind::Delta {
                valley_strength
            } else {
                0.0
            };
            out.delta = out.delta.max(delta_strength);
            let reach = &self.reaches[segment.reach];
            let last = reach.first_segment + reach.segment_count;
            let mouth_distance_l = (1.0 - t) * len2.sqrt()
                + self.segments[k + 1..last]
                    .iter()
                    .map(|s| s.a.point.sub(s.b.point).len())
                    .sum::<f64>();
            let hit = ChannelHit {
                plate_id: self.plate_id,
                segment: k,
                reach: segment.reach,
                distance_l,
                discharge_mm_l2: at.discharge_mm_l2,
                core_l: at.core_l,
                valley_l: influence_l,
                depth_blocks: at.depth_blocks,
                core_strength,
                valley_strength,
                delta_strength,
                end: reach.end,
                mouth_distance_l,
                mirror_y: at.mirror_y,
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
                || !segment.a.mirror_y.is_finite()
                || !segment.b.mirror_y.is_finite()
                || segment.b.mirror_y > segment.a.mirror_y + 1e-9
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

/// T114: на пологом устье моря/озера большая река делится на 2–4 рукава.
/// Главная полилиния укорачивается на длину веера, рукава начинаются в одной
/// точке разветвления; сумма Q рукавов равна Q реки. Это геометрия дельты,
/// не перенос Q между плитами и не игровое поле до T115.
fn append_deltas(
    old_reaches: Vec<ChannelReach>,
    old_segments: Vec<ChannelSegment>,
    seed: u64,
    plate_id: u64,
    params: &Params,
) -> (Vec<ChannelReach>, Vec<ChannelSegment>) {
    let c = &params.channels;
    let delta_seed = subseed(seed, SALT_CHANNEL_DELTA);
    let mut reaches = Vec::with_capacity(old_reaches.len());
    let mut segments = Vec::with_capacity(old_segments.len());
    for (old_index, old) in old_reaches.into_iter().enumerate() {
        let range = old.first_segment..old.first_segment + old.segment_count;
        let last = old_segments[range.end - 1];
        let q = last.b.discharge_mm_l2;
        let eligible = matches!(old.end, ReachEnd::Sea | ReachEnd::Lake)
            && q >= c.delta_q_min_mm_l2
            && last.b.slope_km_l <= c.delta_slope_max;
        let desired = lerp(
            c.delta_length_min_l,
            c.delta_length_max_l,
            clamp((q / c.delta_q_min_mm_l2 - 1.0) / 99.0, 0.0, 1.0),
        );
        if !eligible || old.length_l <= desired + 1e-9 {
            let first_segment = segments.len();
            for mut segment in old_segments[range].iter().copied() {
                segment.reach = reaches.len();
                segments.push(segment);
            }
            reaches.push(ChannelReach {
                first_segment,
                segment_count: old.segment_count,
                ..old
            });
            continue;
        }
        let keep = old.length_l - desired;
        let mut walked = 0.0;
        let first_segment = segments.len();
        let mut apex = last.b;
        let mut direction = last.b.point.sub(last.a.point).normalized();
        for segment in old_segments[range].iter().copied() {
            let len = segment.a.point.sub(segment.b.point).len();
            if walked + len < keep {
                segments.push(ChannelSegment {
                    reach: reaches.len(),
                    ..segment
                });
                walked += len;
                continue;
            }
            let t = if len > 1e-12 {
                clamp((keep - walked) / len, 0.0, 1.0)
            } else {
                0.0
            };
            apex = segment.a.lerp(segment.b, t);
            direction = segment.b.point.sub(segment.a.point).normalized();
            if t > 0.0 {
                segments.push(ChannelSegment {
                    a: segment.a,
                    b: apex,
                    reach: reaches.len(),
                });
            }
            break;
        }
        let segment_count = segments.len() - first_segment;
        let main_index = reaches.len();
        let chord = segments[first_segment].a.point.sub(apex.point).len();
        reaches.push(ChannelReach {
            length_l: keep,
            sinuosity: if chord > 1e-9 { keep / chord } else { 0.0 },
            first_segment,
            segment_count,
            ..old
        });
        let arms =
            2 + (unit(hash3(delta_seed, plate_id as i64, old_index as i64, 0)) * 3.0) as usize;
        let perpendicular = Vec2::new(-direction.z, direction.x);
        for arm in 0..arms {
            let t = if arms == 2 {
                if arm == 0 {
                    -DELTA_TAN_HALF_ANGLE
                } else {
                    DELTA_TAN_HALF_ANGLE
                }
            } else {
                lerp(
                    -DELTA_TAN_HALF_ANGLE,
                    DELTA_TAN_HALF_ANGLE,
                    arm as f64 / (arms - 1) as f64,
                )
            };
            let dir = direction.add(perpendicular.scale(t)).normalized();
            let end = apex.point.add(dir.scale(desired));
            let arm_q = apex.discharge_mm_l2 / arms as f64;
            let mut a = vertex(apex.point, arm_q, apex.slope_km_l, apex.route_km, params);
            let mut b = vertex(end, arm_q, apex.slope_km_l, apex.route_km, params);
            // Рукава дельты наследуют зеркало главной реки в точке ветвления.
            a.mirror_y = apex.mirror_y;
            b.mirror_y = apex.mirror_y;
            let first_segment = segments.len();
            segments.push(ChannelSegment {
                a,
                b,
                reach: reaches.len(),
            });
            reaches.push(ChannelReach {
                source_node: old.mouth_node,
                mouth_node: old.mouth_node,
                end: old.end,
                length_l: desired,
                sinuosity: 1.0,
                plain: old.plain,
                kind: ReachKind::Delta,
                parent_reach: Some(main_index),
                first_segment,
                segment_count: 1,
            });
        }
    }
    (reaches, segments)
}

/// T145: зеркало русла из `route_km`, округлённое до блока и не растущее
/// вниз по течению. Ступень не больше 1 блока; на равнине шаги не чаще
/// `mirror_step_blocks`, на крутых участках — каскад с меньшим шагом.
/// Если рельеф падает быстрее, зеркало остаётся выше сырого уровня: русло
/// прорезается под него, а берег ограничивает воду уже в height::with_water.
fn assign_mirror(points: &mut [ChannelPoint], plain: bool, params: &Params) {
    let Some(last) = points.last_mut() else {
        return;
    };
    let scale = params.height.water_km_to_blocks;
    let step = if plain {
        params.channels.mirror_step_blocks
    } else {
        params.channels.mirror_cascade_step_blocks
    }
    .max(1.0);
    let mut y = (crate::height::SEA_LEVEL + last.route_km.max(0.0) * scale).round();
    last.mirror_y = y;
    let l = params.block.l_blocks;
    for i in (0..points.len() - 1).rev() {
        let dist_blocks = points[i].point.sub(points[i + 1].point).len() * l;
        let raw = (crate::height::SEA_LEVEL + points[i].route_km.max(0.0) * scale).round();
        let allowed = (dist_blocks / step).floor().max(0.0);
        // Вверх по течению зеркало не ниже downstream, но поднимается не
        // быстрее одной ступени на разрешённый интервал.
        let target = raw.max(y).min(y + allowed);
        points[i].mirror_y = target;
        y = target;
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
        c.delta_q_min_mm_l2,
        c.delta_slope_max,
        c.delta_length_min_l,
        c.delta_length_max_l,
        c.mirror_step_blocks,
        c.mirror_cascade_step_blocks,
    ];
    if values.iter().any(|v| !v.is_finite() || *v <= 0.0)
        || c.core_max_blocks < c.core_min_blocks
        || c.valley_plain < 1.0
        || c.valley_mountain < 1.0
        || c.delta_length_max_l < c.delta_length_min_l
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

fn vertex(point: Vec2, q: f64, slope: f64, route_km: f64, params: &Params) -> ChannelPoint {
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
        route_km,
        mirror_y: (crate::height::SEA_LEVEL + route_km.max(0.0) * params.height.water_km_to_blocks)
            .round(),
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
        out.push(vertex(nodes[k].point, q, slope, nodes[k].route_km, params));
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
        // Д-057: меандр — синусоида по фазе вдоль русла, а шум только сбивает
        // фазу и меняет размах. Чистый градиентный шум давал извилистость
        // 1,03: его типичный размах втрое меньше амплитуды.
        let wobble = 0.35 * noise2(seed, 0.5 * phase, offset);
        let gain = clamp(
            0.8 + 0.6 * noise2(seed, 0.37 * phase, offset + 17.0),
            0.4,
            1.2,
        );
        let cycle = phase + wobble;
        let angle = (cycle - cycle.floor() - 0.5) * 2.0 * std::f64::consts::PI;
        let wave = -crate::math::rotate_unit(Vec2::new(1.0, 0.0), angle).z;
        let shift = tangent
            .perp()
            .scale(input[k].meander_amplitude_l * wave * gain * envelope);
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
        // Механика русел проверяется на пороге Q первого набора: подбор
        // густоты сети (Д-057) не должен менять геометрию этих тестов.
        let mut p = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        p.channels.q_min_mm_l2 = 25.0;
        p
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
                t_min_c: 10.0,
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
                let fade = (net.bank_fraction * at.core_l).max(net.field_fade_l);
                let gallery_l = at.core_l * net.gallery_width_factor;
                let influence_l = at.valley_l.max(at.core_l + fade).max(gallery_l);
                if d <= influence_l {
                    let core = 1.0 - crate::math::smoothstep(at.core_l, at.core_l + fade, d);
                    let valley = 1.0 - crate::math::smoothstep(0.0, influence_l, d);
                    brute.core = brute.core.max(core);
                    brute.valley = brute.valley.max(valley);
                    let large = crate::math::smoothstep(
                        0.5 * net.gallery_q_min_mm_l2,
                        net.gallery_q_min_mm_l2,
                        at.discharge_mm_l2,
                    );
                    brute.gallery = brute
                        .gallery
                        .max(large * (1.0 - crate::math::smoothstep(0.0, gallery_l, d)));
                    let delta = if net.reaches()[segment.reach].kind == super::ReachKind::Delta {
                        valley
                    } else {
                        0.0
                    };
                    brute.delta = brute.delta.max(delta);
                    if brute
                        .nearest
                        .is_none_or(|old: super::ChannelHit| d < old.distance_l)
                    {
                        brute.nearest = Some(super::ChannelHit {
                            plate_id: 7,
                            segment: s,
                            reach: segment.reach,
                            distance_l: d,
                            discharge_mm_l2: at.discharge_mm_l2,
                            core_l: at.core_l,
                            valley_l: influence_l,
                            depth_blocks: at.depth_blocks,
                            core_strength: core,
                            valley_strength: valley,
                            delta_strength: delta,
                            end: net.reaches()[segment.reach].end,
                            mirror_y: at.mirror_y,
                            mouth_distance_l: (1.0 - t) * ab.dot(ab).sqrt()
                                + net
                                    .reach_segments(segment.reach)
                                    .iter()
                                    .skip(s - net.reach_first_segment(segment.reach))
                                    .skip(1)
                                    .map(|s| s.a.point.sub(s.b.point).len())
                                    .sum::<f64>(),
                        });
                    }
                }
            }
            assert_eq!(fast.core, brute.core);
            assert_eq!(fast.valley, brute.valley);
            assert_eq!(fast.gallery, brute.gallery);
            assert_eq!(fast.nearest, brute.nearest);
            assert_eq!(fast.networks_considered, 1);
            assert_eq!(fast, net.query(point), "point={point:?} basket={BASKET_L}");
        }
    }

    #[test]
    fn ring_outlets_keep_their_kind_and_queries_merge_deterministically() {
        let p = params();
        for (outlet, end) in [
            (DrainageOutlet::Sea, ReachEnd::Sea),
            (DrainageOutlet::Rift, ReachEnd::Rift),
            (DrainageOutlet::Transform, ReachEnd::Transform),
        ] {
            let mut mouth = node(0, 0.0, None);
            mouth.in_plate = false;
            mouth.outlet = outlet;
            let nodes = vec![node(1, 1.0, Some(1)), mouth];
            let flow = runoff(&nodes, &p, 4.0);
            let net = ChannelNetwork::build(7, 42, &nodes, &flow, &p).unwrap();
            assert_eq!(net.reaches()[0].end, end);
        }
        let nodes = vec![node(4, 4.0, Some(1)), node(3, 3.0, None)];
        let flow = runoff(&nodes, &p, 4.0);
        let a = ChannelNetwork::build(7, 42, &nodes, &flow, &p).unwrap();
        let b = ChannelNetwork::build(8, 42, &nodes, &flow, &p).unwrap();
        let point = Vec2::new(0.875, 0.0);
        let qa = a.query(point);
        let qb = b.query(point);
        let mut ab = qa;
        ab.merge(qb);
        let mut ba = qb;
        ba.merge(qa);
        assert_eq!(ab, ba);
        assert_eq!(ab.networks_considered, 2);
        assert_eq!(ab.nearest.unwrap().plate_id, 7);
        assert!(super::max_query_radius_l(&p) >= 0.56 + 0.336);
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

    #[test]
    fn gallery_corridor_is_gated_by_discharge_and_fades_with_distance() {
        let p = params();
        let nodes = vec![
            node(3, 0.003, Some(1)),
            node(2, 0.002, Some(2)),
            node(1, 0.001, Some(3)),
            node(0, 0.0, None),
        ];
        let small = runoff(&nodes, &p, 4.0);
        let large = runoff(&nodes, &p, 100.0);
        let small = ChannelNetwork::build(7, 42, &nodes, &small, &p).unwrap();
        let large = ChannelNetwork::build(7, 42, &nodes, &large, &p).unwrap();
        let main = large
            .reaches()
            .iter()
            .find(|r| r.kind == super::ReachKind::Channel)
            .unwrap();
        let segment = large.segments()[main.first_segment + main.segment_count - 1];
        let at = segment.b.point;
        let core = large.query(at);
        let edge = large.query(at.add(Vec2::new(
            0.0,
            segment.b.core_l * p.hydrology.gallery_width_factor * 0.75,
        )));
        assert_eq!(small.query(at).gallery, 0.0);
        assert!(core.gallery > 0.9);
        assert!(edge.gallery < core.gallery && edge.gallery > 0.0);
        assert_eq!(
            large
                // Запас на размах меандра: русло выше устья качается поперёк
                // оси до 2·meander·1.2 ядра.
                .query(at.add(Vec2::new(
                    0.0,
                    segment.b.core_l
                        * (p.hydrology.gallery_width_factor * 1.1
                            + 2.4 * p.channels.meander_plain.max(p.channels.meander_mountain))
                )))
                .gallery,
            0.0
        );
    }

    #[test]
    fn large_gentle_mouth_splits_into_two_to_four_delta_arms() {
        let p = params();
        let nodes = vec![
            node(3, 0.003, Some(1)),
            node(2, 0.002, Some(2)),
            node(1, 0.001, Some(3)),
            node(0, 0.0, None),
        ];
        let flow = runoff(&nodes, &p, 100.0);
        let net = ChannelNetwork::build(7, 42, &nodes, &flow, &p).unwrap();
        assert_eq!(net.delta_count(), 1);
        let arms: Vec<_> = net
            .reaches()
            .iter()
            .filter(|r| r.kind == super::ReachKind::Delta)
            .collect();
        assert!((2..=4).contains(&arms.len()));
        let main = &net.reaches()[arms[0].parent_reach.unwrap()];
        assert!(main.length_l > 0.0);
        let total_q: f64 = arms
            .iter()
            .map(|r| net.segments()[r.first_segment].a.discharge_mm_l2)
            .sum();
        let mouth_q = net.segments()[main.first_segment + main.segment_count - 1]
            .b
            .discharge_mm_l2;
        assert!((total_q - mouth_q).abs() < 1e-9);
        let axis = arms
            .iter()
            .fold(Vec2::default(), |acc, arm| {
                let segment = net.segments()[arm.first_segment];
                acc.add(segment.b.point.sub(segment.a.point).normalized())
            })
            .normalized();
        for arm in arms {
            assert!(arm.length_l >= p.channels.delta_length_min_l - 1e-12);
            assert!(arm.length_l <= p.channels.delta_length_max_l + 1e-12);
            let segment = net.segments()[arm.first_segment];
            let dir = segment.b.point.sub(segment.a.point).normalized();
            let forward = dir.dot(axis);
            let sideways = dir.dot(axis.perp()).abs();
            assert!(sideways <= forward * super::DELTA_TAN_HALF_ANGLE + 1e-9);
            let mid = segment
                .a
                .point
                .add(segment.b.point.sub(segment.a.point).scale(0.5));
            assert!(net.query(mid).delta > 0.0);
        }
    }

    #[test]
    fn steep_or_small_mouths_do_not_grow_deltas() {
        let p = params();
        let nodes = vec![
            node(3, 3.0, Some(1)),
            node(2, 2.0, Some(2)),
            node(1, 1.0, Some(3)),
            node(0, 0.0, None),
        ];
        let steep = runoff(&nodes, &p, 100.0);
        assert_eq!(
            ChannelNetwork::build(7, 42, &nodes, &steep, &p)
                .unwrap()
                .delta_count(),
            0
        );
        let small = runoff(&nodes, &p, 0.1);
        assert_eq!(
            ChannelNetwork::build(7, 42, &nodes, &small, &p)
                .unwrap()
                .delta_count(),
            0
        );
    }
}

#[cfg(test)]
mod mirror_tests {
    use super::{ChannelPoint, assign_mirror, vertex};
    use crate::math::Vec2;
    use crate::mode::Mode;
    use crate::params::Params;
    use crate::version::GenVersion;

    fn points(routes: &[f64], params: &Params) -> Vec<ChannelPoint> {
        routes
            .iter()
            .enumerate()
            .map(|(i, &route)| vertex(Vec2::new(i as f64 * 0.5, 0.0), 100.0, 0.01, route, params))
            .collect()
    }

    #[test]
    fn mirror_is_rounded_monotone_and_step_limited() {
        let p = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        let mut pts = points(&[0.3, 0.2, 0.1, 0.0], &p);
        assign_mirror(&mut pts, true, &p);
        for pair in pts.windows(2) {
            assert!(pair[1].mirror_y <= pair[0].mirror_y);
            assert_eq!(pair[0].mirror_y.fract(), 0.0);
            let dist = pair[0].point.sub(pair[1].point).len() * p.block.l_blocks;
            let allowed = (dist / p.channels.mirror_step_blocks).floor();
            assert!(pair[0].mirror_y - pair[1].mirror_y <= allowed);
        }
        assert_eq!(pts.last().unwrap().mirror_y, crate::height::SEA_LEVEL);
    }

    #[test]
    fn adverse_raw_slope_does_not_make_water_flow_uphill() {
        let p = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        let mut pts = points(&[0.0, 0.1], &p);
        assign_mirror(&mut pts, true, &p);
        assert!(pts[1].mirror_y <= pts[0].mirror_y);
        assert_eq!(pts[0].mirror_y, pts[1].mirror_y);
    }
}
