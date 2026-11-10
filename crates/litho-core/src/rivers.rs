//! Реки «карандашом» (план 7.10, задачи T50/T51).
//!
//! В v0 рифтовые и предгорные реки берутся из весов границы плиты. В v1-dev
//! T113 оставляет рифт и трансформ, а предгорную трассу убирает (§5.5). Трассы от
//! горных истоков считаются один раз на ячейку сетки истоков и кэшируются:
//! иначе эталонный расчёт в каждой точке стоил бы дороже всего остального.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::climate::SurfaceSample;
use crate::hash::{hash3, splitmix64, unit};
use crate::math::{Vec2, clamp01, smoothstep};
use crate::noise::noise2;
use crate::params::Params;
use crate::seed::{SALT_RIVERS, SALT_RIVERS_MEANDER, SALT_SPAWN, cell_hash, subseed};

#[derive(Clone, Copy, Debug)]
struct Segment {
    a: Vec2,
    b: Vec2,
    half_width_blocks: f64,
}

/// Кэш трасс по номеру ячейки истока. Значения от порядка вычислений не
/// зависят: трасса детерминирована, кэш только экономит повторную работу.
type TraceMap = HashMap<(i64, i64), Arc<Vec<Segment>>>;

#[derive(Debug)]
pub struct RiverCache {
    traces: Mutex<TraceMap>,
}

impl RiverCache {
    pub fn new() -> Self {
        Self {
            traces: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for RiverCache {
    fn default() -> Self {
        Self::new()
    }
}

fn source_in_cell(seed: u64, i: i64, j: i64, params: &Params) -> Vec2 {
    let g = params.rivers.source_cell_l;
    let inset = 0.1;
    let h = cell_hash(subseed(seed, SALT_RIVERS), i, j, 0);
    let h2 = splitmix64(h);
    Vec2::new(
        (i as f64 + inset + (1.0 - 2.0 * inset) * unit(h)) * g,
        (j as f64 + inset + (1.0 - 2.0 * inset) * unit(h2)) * g,
    )
}

/// Поворот на малый угол без системных `sin`/`cos`: разложение Тейлора
/// достаточно точно при |a| ≤ 0,6 рад и побитово одинаково на всех платформах.
fn rotated(v: Vec2, angle: f64) -> Vec2 {
    let cos_a = 1.0 - 0.5 * angle * angle;
    let sin_a = angle - angle * angle * angle / 6.0;
    Vec2::new(v.x * cos_a - v.z * sin_a, v.x * sin_a + v.z * cos_a).normalized()
}

fn distance_to_segment(point: Vec2, a: Vec2, b: Vec2) -> f64 {
    let ab = b.sub(a);
    let len2 = ab.dot(ab);
    if len2 <= 1e-12 {
        return point.sub(a).len();
    }
    let t = clamp01(point.sub(a).dot(ab) / len2);
    let projection = a.add(ab.scale(t));
    point.sub(projection).len()
}

fn source_is_good(
    source: Vec2,
    seed: u64,
    params: &Params,
    surface: &impl Fn(Vec2) -> SurfaceSample,
) -> bool {
    let r = &params.rivers;
    let s = surface(source);
    if s.mountain < r.source_min_mountain || s.e_km < r.source_min_height_km {
        return false;
    }
    let h = hash3(subseed(seed, SALT_SPAWN), 0, 0, 0);
    let phi = crate::climate::pole_sample(source, seed, params).map_or(45.0, |p| p.phi_deg);
    let p_mm = crate::climate::precipitation(phi, 0.5, 0.6, 0.0, params);
    let accepted =
        unit(h.wrapping_add(source.x.to_bits() ^ source.z.to_bits())) < r.source_probability;
    accepted && p_mm >= r.source_min_precip_mm
}

fn trace_source<F>(source: Vec2, seed: u64, params: &Params, surface: &F) -> Vec<Segment>
where
    F: Fn(Vec2) -> SurfaceSample,
{
    let r = &params.rivers;
    if !source_is_good(source, seed, params, surface) {
        return Vec::new();
    }
    let step = r.trace_step_l;
    let grad_step = r.gradient_step_l;
    let meander_seed = subseed(seed, SALT_RIVERS_MEANDER);
    let mut points = Vec::with_capacity(r.trace_max_steps + 1);
    points.push(source);
    let mut direction = Vec2::new(0.0, 1.0);
    let mut flat_steps = 0_usize;
    for _ in 0..r.trace_max_steps {
        let p = *points.last().expect("трасса не пустая");
        if surface(p).e_km < 0.0 {
            break;
        }
        let dx = (surface(p.add(Vec2::new(grad_step, 0.0))).e_km
            - surface(p.add(Vec2::new(-grad_step, 0.0))).e_km)
            / (2.0 * grad_step);
        let dz = (surface(p.add(Vec2::new(0.0, grad_step))).e_km
            - surface(p.add(Vec2::new(0.0, -grad_step))).e_km)
            / (2.0 * grad_step);
        let gradient = Vec2::new(dx, dz);
        let slope = gradient.len();
        if slope < r.trace_gradient_min {
            flat_steps += 1;
            if flat_steps >= r.trace_flat_steps {
                break;
            }
        } else {
            flat_steps = 0;
        }
        let downhill = if slope > 1e-9 {
            gradient.scale(-1.0 / slope)
        } else {
            direction
        };
        let blended = direction.scale(0.5).add(downhill.scale(0.5)).normalized();
        let angle = r.meander_turn_rad * noise2(meander_seed, p.x / 0.5, p.z / 0.5);
        direction = rotated(blended, angle);
        points.push(p.add(direction.scale(step)));
    }
    let length = (points.len() - 1) as f64 * step;
    if length < r.trace_min_length_l {
        return Vec::new();
    }
    let mut segments = Vec::with_capacity(points.len() - 1);
    for (k, pair) in points.windows(2).enumerate() {
        let distance = k as f64 * step;
        let width = params.block.river_half_width_blocks
            * (1.0 + r.half_width_growth * (distance / r.half_width_length_l).min(1.0));
        segments.push(Segment {
            a: pair[0],
            b: pair[1],
            half_width_blocks: width,
        });
    }
    segments
}

/// Дешёвая река границы плиты. v0 сохраняет рифт + предгорье (7.10.1);
/// v1-dev использует рифт + трансформ с той же постоянной шириной (T113).
/// `σ` берём как расстояние до ближайшей границы плиты.
pub fn local_r(x: f64, z: f64, surface: &SurfaceSample, seed: u64, params: &Params) -> f64 {
    if surface.e_km < 0.0 {
        return 0.0;
    }
    let r = &params.rivers;
    let l = params.block.l_blocks;
    let width = params.block.river_half_width_blocks;
    let amplitude = params.block.meander_amplitude_blocks;
    let lambda = params.block.meander_lambda_blocks;
    let meander = amplitude * noise2(subseed(seed, SALT_RIVERS_MEANDER), x / lambda, z / lambda);
    let sigma = surface.sigma_l;
    let k = surface.boundary_weights;
    let distance = (sigma * l + meander).abs();
    let rift = k[4] * (1.0 - smoothstep(0.0, width, distance));
    if params.version.is_water_relief_dev() {
        let transform = k[6] * (1.0 - smoothstep(0.0, width, distance));
        return clamp01(rift.max(transform));
    }
    let sigma_f = r.foothill_sigma_factor * params.crust.w_col_l;
    let foot = k[0] * (1.0 - smoothstep(0.0, width, ((sigma - sigma_f) * l + meander).abs()));
    clamp01(rift.max(foot))
}

/// Сила трассированных рек: максимум по отрезкам трасс из окна ячеек истоков.
pub fn traced_r<F>(
    point: Vec2,
    e_km: f64,
    seed: u64,
    params: &Params,
    surface: &F,
    cache: &RiverCache,
) -> f64
where
    F: Fn(Vec2) -> SurfaceSample,
{
    if e_km < 0.0 {
        return 0.0;
    }
    let r = &params.rivers;
    let g = r.source_cell_l;
    let ci = (point.x / g).floor() as i64;
    let cj = (point.z / g).floor() as i64;
    let radius = r.source_window_cells;
    let l = params.block.l_blocks;
    let mut best = 0.0_f64;
    for di in -radius..=radius {
        for dj in -radius..=radius {
            let key = (ci + di, cj + dj);
            let segments = {
                let mut map = cache.traces.lock().expect("кэш рек не отравлен");
                map.entry(key)
                    .or_insert_with(|| {
                        Arc::new(trace_source(
                            source_in_cell(seed, key.0, key.1, params),
                            seed,
                            params,
                            surface,
                        ))
                    })
                    .clone()
            };
            for segment in segments.iter() {
                let distance_blocks = distance_to_segment(point, segment.a, segment.b) * l;
                let value = 1.0 - smoothstep(0.0, segment.half_width_blocks, distance_blocks);
                best = best.max(value);
            }
        }
    }
    clamp01(best)
}

#[cfg(test)]
mod tests {
    use super::local_r;
    use crate::climate::SurfaceSample;
    use crate::mode::Mode;
    use crate::params::Params;
    use crate::version::GenVersion;

    fn surface(weights: [f64; 7], sigma_l: f64) -> SurfaceSample {
        SurfaceSample {
            e_km: 0.5,
            boundary_weights: weights,
            sigma_l,
            ..SurfaceSample::default()
        }
    }

    #[test]
    fn dev_keeps_rift_adds_transform_and_drops_foothill_without_touching_v0() {
        let dev = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        let frozen = Params::defaults(GenVersion::V0, Mode::Gameplay);
        let mut transform = [0.0; 7];
        transform[6] = 1.0;
        let mut foothill = [0.0; 7];
        foothill[0] = 1.0;
        let sigma_f = frozen.rivers.foothill_sigma_factor * frozen.crust.w_col_l;
        assert!(local_r(0.0, 0.0, &surface(transform, 0.0), 42, &dev) > 0.9);
        assert_eq!(
            local_r(0.0, 0.0, &surface(transform, 0.0), 42, &frozen),
            0.0
        );
        assert_eq!(
            local_r(0.0, 0.0, &surface(foothill, sigma_f), 42, &dev),
            0.0
        );
        assert!(local_r(0.0, 0.0, &surface(foothill, sigma_f), 42, &frozen) > 0.0);
        let mut rift = [0.0; 7];
        rift[4] = 1.0;
        for k in 0..16 {
            let x = k as f64 * 11.0;
            let s = surface(rift, 0.01);
            assert_eq!(
                local_r(x, -x, &s, 42, &dev).to_bits(),
                local_r(x, -x, &s, 42, &frozen).to_bits()
            );
        }
        assert_eq!(local_r(0.0, 0.0, &surface(transform, 2.0), 42, &dev), 0.0);
    }
}
