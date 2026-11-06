//! Литосферные плиты (план 7.3): семена, ячейки Вороного, типы границ.
//!
//! Значение в точке зависит только от семян в окне 5×5 ячеек вокруг точки, то
//! есть локально (6.4). Здесь эталонный путь с `Vec` кандидатов; быстрый путь с
//! кэшем тайлов появится в T13/T14/T55, формулы при этом не изменятся.

use crate::hash::{hash3, splitmix64, unit};
use crate::math::{Vec2, clamp01, dexp};
use crate::noise::warp2;
use crate::noise_tables::DIRS;
use crate::params::Params;
use crate::seed::{
    CellRng, SALT_PLATES_ID, SALT_PLATES_PROPERTIES, SALT_PLATES_SEED, SALT_PLATES_VELOCITY,
    SALT_WARP_PLATES_X, SALT_WARP_PLATES_Z, cell_hash, subseed,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plate {
    pub id: u64,
    pub cell_i: i64,
    pub cell_j: i64,
    pub index: i64,
    pub seed_pos: Vec2,
    pub center: Vec2,
    pub has_continent: bool,
    pub radius_l: f64,
    pub aspect: f64,
    pub axis: Vec2,
    pub velocity: Vec2,
    pub omega: f64,
    pub age: f64,
}

impl Plate {
    /// Скорость плиты в точке `q`: поступательная плюс вращение вокруг семени.
    /// Вращение нужно, чтобы тип границы менялся вдоль длинной границы.
    pub fn velocity_at(&self, q: Vec2) -> Vec2 {
        self.velocity
            .add(q.sub(self.seed_pos).perp().scale(self.omega))
    }
}

/// Результат запроса плиты в точке: ближайшая плита A, две ближайшие «чужие»
/// B и C, расстояния до их серединных перпендикуляров и нормали.
#[derive(Clone, Copy, Debug)]
pub struct PlateSample {
    pub a: Plate,
    pub b: Option<Plate>,
    pub c: Option<Plate>,
    pub d_ab_l: f64,
    pub d_ac_l: f64,
    pub n_ab: Vec2,
    pub n_ac: Vec2,
    pub w_b: f64,
    pub w_c: f64,
    pub q: Vec2,
}

impl PlateSample {
    /// Расстояние до ближайшей границы плиты A (макро-единицы).
    pub const fn nearest_boundary_l(&self) -> f64 {
        self.d_ab_l
    }

    pub fn plate_id(&self) -> u64 {
        self.a.id
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoundaryType {
    pub conv: f64,
    pub div: f64,
    pub tr: f64,
}

impl BoundaryType {
    pub fn dominant(&self) -> &'static str {
        if self.conv >= self.div && self.conv >= self.tr {
            "convergent"
        } else if self.div >= self.tr {
            "divergent"
        } else {
            "transform"
        }
    }
}

/// Соль `warp.plates.*` → сдвиг точки перед поиском плиты (7.3.2).
pub fn warped_point(point: Vec2, seed: u64, params: &Params) -> Vec2 {
    let p = &params.plates;
    let sx = subseed(seed, SALT_WARP_PLATES_X);
    let sz = subseed(seed, SALT_WARP_PLATES_Z);
    let w = warp2(
        sx,
        sz,
        point,
        p.warp_lambda0_l,
        p.warp_amplitude_l,
        p.warp_octaves,
        p.warp_gain,
    );
    point.add(w)
}

/// Якобиан отображения `p → q`. Если он становится отрицательным, плоскость
/// «складывается» и появляются оторванные куски плит (7.3.2).
pub fn warp_jacobian(point: Vec2, seed: u64, params: &Params) -> f64 {
    let h = 0.01;
    let px = warped_point(point.add(Vec2::new(h, 0.0)), seed, params);
    let mx = warped_point(point.sub(Vec2::new(h, 0.0)), seed, params);
    let pz = warped_point(point.add(Vec2::new(0.0, h)), seed, params);
    let mz = warped_point(point.sub(Vec2::new(0.0, h)), seed, params);
    let dx = px.sub(mx).scale(1.0 / (2.0 * h));
    let dz = pz.sub(mz).scale(1.0 / (2.0 * h));
    dx.x * dz.z - dx.z * dz.x
}

/// Семена плит в одной ячейке сетки `G = 16 L`.
///
/// Порядок случайных чисел (фиксирован, менять нельзя):
/// `h0` из `hash3(plates.seed, i, j, 0)` даёт `r_empty`, `r_double`, затем
/// по две координаты на каждую попытку поставить семя. Отдельно для каждого
/// семени `k`: `properties` даёт смещение центра, долю суши, радиус,
/// вытянутость, возраст и ось эллипса; `velocity` даёт направление, модуль
/// скорости и угловую скорость; `id` даёт идентификатор плиты.
pub fn seeds_in_cell(seed: u64, i: i64, j: i64, params: &Params) -> Vec<Plate> {
    let p = &params.plates;
    let g = p.cell_g_l;
    let mut rng = CellRng::from_hash(cell_hash(subseed(seed, SALT_PLATES_SEED), i, j, 0));

    let even_cell = i.rem_euclid(2) == 0 && j.rem_euclid(2) == 0;
    let r_empty = rng.next_unit();
    if even_cell && r_empty < p.empty_probability {
        return Vec::new();
    }
    let r_double = rng.next_unit();
    let wanted = if r_double < p.double_probability {
        2
    } else {
        1
    };
    let inset = p.inset_fraction;

    let mut positions: Vec<Vec2> = Vec::with_capacity(2);
    let first = Vec2::new(
        (i as f64 + rng.next_range(inset, 1.0 - inset)) * g,
        (j as f64 + rng.next_range(inset, 1.0 - inset)) * g,
    );
    positions.push(first);
    if wanted == 2 {
        let min_dist = p.min_seed_distance_fraction * g;
        let min_dist2 = min_dist * min_dist;
        for _ in 0..p.seed_attempts {
            let candidate = Vec2::new(
                (i as f64 + rng.next_range(inset, 1.0 - inset)) * g,
                (j as f64 + rng.next_range(inset, 1.0 - inset)) * g,
            );
            if candidate.dist2(first) >= min_dist2 {
                positions.push(candidate);
                break;
            }
        }
    }

    let mut result = Vec::with_capacity(positions.len());
    for (k, seed_pos) in positions.iter().enumerate() {
        let k_i64 = k as i64;
        let mut props = CellRng::from_hash(cell_hash(
            subseed(seed, SALT_PLATES_PROPERTIES),
            i,
            j,
            k_i64,
        ));
        let offset = p.center_offset_fraction * g;
        let center = seed_pos.add(Vec2::new(
            props.next_range(-offset, offset),
            props.next_range(-offset, offset),
        ));
        let has_continent = props.next_unit() < p.continent_probability;
        let radius_l = props.next_range(p.continent_radius_min_l, p.continent_radius_max_l);
        let aspect = props.next_range(p.aspect_min, p.aspect_max);
        let age = props.next_unit();
        let axis = dir_from_rng(&mut props);

        let mut motion =
            CellRng::from_hash(cell_hash(subseed(seed, SALT_PLATES_VELOCITY), i, j, k_i64));
        let velocity = dir_from_rng(&mut motion).scale(motion.next_range(p.speed_min, p.speed_max));
        let omega = motion.next_range(-p.omega_max, p.omega_max);

        result.push(Plate {
            id: cell_hash(subseed(seed, SALT_PLATES_ID), i, j, k_i64),
            cell_i: i,
            cell_j: j,
            index: k_i64,
            seed_pos: *seed_pos,
            center,
            has_continent,
            radius_l,
            aspect,
            axis,
            velocity,
            omega,
            age,
        });
    }
    result
}

fn dir_from_rng(rng: &mut CellRng) -> Vec2 {
    let index = (rng.next_u64() & 255) as usize;
    let d = DIRS[index];
    Vec2::new(d[0], d[1])
}

/// Поиск плиты в точке. `radius_cells = 2` даёт окно 5×5 (рабочий вариант);
/// большее окно нужно только тестам и отладке.
pub fn plate_query_window(
    point: Vec2,
    seed: u64,
    params: &Params,
    radius_cells: i64,
) -> Option<PlateSample> {
    let p = &params.plates;
    let q = warped_point(point, seed, params);
    let ci = (q.x / p.cell_g_l).floor() as i64;
    let cj = (q.z / p.cell_g_l).floor() as i64;

    let mut candidates: Vec<Plate> = Vec::with_capacity(64);
    for dj in -radius_cells..=radius_cells {
        for di in -radius_cells..=radius_cells {
            for plate in seeds_in_cell(seed, ci + di, cj + dj, params) {
                candidates.push(plate);
            }
        }
    }
    if candidates.is_empty() {
        return None;
    }

    let mut best = 0;
    let mut best_d2 = f64::INFINITY;
    for (k, plate) in candidates.iter().enumerate() {
        let d2 = q.dist2(plate.seed_pos);
        if d2 < best_d2 {
            best_d2 = d2;
            best = k;
        }
    }
    let a = candidates[best];

    // β_k — расстояние до серединного перпендикуляра между A и k.
    let mut first: Option<(f64, Plate, Vec2)> = None;
    let mut second: Option<(f64, Plate, Vec2)> = None;
    for (k, plate) in candidates.iter().enumerate() {
        if k == best {
            continue;
        }
        let delta = plate.seed_pos.sub(a.seed_pos);
        let separation = delta.len();
        if separation <= 0.0 {
            continue;
        }
        let beta = (q.dist2(plate.seed_pos) - best_d2) / (2.0 * separation);
        let normal = delta.scale(1.0 / separation);
        let entry = (beta, *plate, normal);
        match first {
            None => first = Some(entry),
            Some((d0, _, _)) if beta < d0 => {
                second = first;
                first = Some(entry);
            }
            Some(_) => match second {
                None => second = Some(entry),
                Some((d1, _, _)) if beta < d1 => second = Some(entry),
                Some(_) => {}
            },
        }
    }

    let (d_ab_l, b, n_ab) = match first {
        Some((d, plate, n)) => (d.max(0.0), Some(plate), n),
        None => (f64::INFINITY, None, Vec2::default()),
    };
    let (d_ac_l, c, n_ac) = match second {
        Some((d, plate, n)) => (d.max(0.0), Some(plate), n),
        None => (f64::INFINITY, None, Vec2::default()),
    };

    let wb = dexp(-d_ab_l / p.triple_weight_l);
    let wc = dexp(-d_ac_l / p.triple_weight_l);
    let total = wb + wc;
    let (w_b, w_c) = if total > 0.0 {
        (wb / total, wc / total)
    } else {
        (0.5, 0.5)
    };

    Some(PlateSample {
        a,
        b,
        c,
        d_ab_l,
        d_ac_l,
        n_ab,
        n_ac,
        w_b,
        w_c,
        q,
    })
}

/// Рабочий запрос: окно 5×5, как в 7.3.3.
pub fn plate_query(point: Vec2, seed: u64, params: &Params) -> Option<PlateSample> {
    plate_query_window(point, seed, params, 2)
}

/// Тип границы между A и B в точке `q` (7.3.5). `n_ab` — нормаль от A к B.
/// Формула симметрична: с точки зрения B получаются те же `conv`, `div`, `tr`.
pub fn boundary_type(a: &Plate, b: &Plate, q: Vec2, n_ab: Vec2, params: &Params) -> BoundaryType {
    let p = &params.plates;
    let dv = a.velocity_at(q).sub(b.velocity_at(q));
    let c = dv.dot(n_ab);
    let tau = (dv.x * n_ab.z - dv.z * n_ab.x).abs();
    let width = p.boundary_width;
    let conv = clamp01((c - p.boundary_conv_threshold) / width);
    let div = clamp01((-c - p.boundary_conv_threshold) / width);
    let tr = clamp01((tau - p.boundary_shear_threshold) / width) * (1.0 - conv.max(div));
    BoundaryType { conv, div, tr }
}

/// Веса `k1…k7` классификации границ (7.3.6). `chi_a`, `chi_b` — доля
/// материковой коры у границы, 0 = океан, 1 = материк.
pub fn boundary_weights(boundary: BoundaryType, chi_a: f64, chi_b: f64) -> [f64; 7] {
    let both_land = chi_a * chi_b;
    let mixed = (chi_a - chi_b).abs();
    let both_ocean = (1.0 - chi_a) * (1.0 - chi_b);
    [
        boundary.conv * both_land,
        boundary.conv * mixed,
        boundary.conv * both_ocean,
        boundary.div * both_ocean,
        boundary.div * both_land,
        boundary.div * mixed,
        boundary.tr,
    ]
}

/// Приближённая доля материковой коры плиты (7.4.1) нужна уже здесь, чтобы
/// типы границ «материк–материк» и «материк–океан» различались на карте плит.
/// Полная версия с шумом и смешиванием живёт в `crust.rs` (T22).
pub fn continent_field(plate: &Plate, q: Vec2) -> f64 {
    if !plate.has_continent {
        return -6.0;
    }
    let rel = q.sub(plate.center);
    let u = rel.dot(plate.axis);
    let w = rel.dot(plate.axis.perp());
    let r = plate.radius_l;
    let a = r * plate.aspect.sqrt();
    let b = r / plate.aspect.sqrt();
    let rho = ((u / a) * (u / a) + (w / b) * (w / b)).sqrt();
    r * (1.0 - rho)
}

/// Плавная доля материка `χ` для весов границы.
pub fn continent_fraction(kappa_l: f64) -> f64 {
    crate::math::smoothstep(-0.5, 0.5, kappa_l)
}

/// Случайный цвет плиты для слоя `plates` песочницы.
pub fn plate_color(id: u64) -> [u8; 3] {
    let h0 = hash3(id, 0, 0, 0);
    let h1 = splitmix64(h0);
    let h2 = splitmix64(h1);
    [
        100 + (unit(h0) * 145.0) as u8,
        100 + (unit(h1) * 145.0) as u8,
        100 + (unit(h2) * 145.0) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::{
        boundary_type, continent_field, continent_fraction, plate_query, plate_query_window,
        seeds_in_cell, warp_jacobian,
    };
    use crate::hash::{splitmix64, unit};
    use crate::math::Vec2;
    use crate::mode::Mode;
    use crate::params::Params;
    use crate::version::GenVersion;

    fn params() -> Params {
        Params::defaults(GenVersion::V0, Mode::Gameplay)
    }

    #[test]
    fn empty_cells_only_at_even_indices() {
        let p = params();
        for i in -6..6 {
            for j in -6..6 {
                let seeds = seeds_in_cell(42, i, j, &p);
                if i.rem_euclid(2) != 0 || j.rem_euclid(2) != 0 {
                    assert!(!seeds.is_empty(), "ячейка ({i}, {j}) обязана иметь семя");
                }
                assert!(seeds.len() <= 2);
            }
        }
    }

    #[test]
    fn window_5x5_matches_brute_force_9x9() {
        let p = params();
        for seed in [0_i64, 42, -123_456_789] {
            let s = seed as u64;
            for k in 0..300 {
                let x = unit(splitmix64(k)) * 400.0 - 200.0;
                let z = unit(splitmix64(k + 100_000)) * 400.0 - 200.0;
                let point = Vec2::new(x, z);
                let small = plate_query_window(point, s, &p, 2).expect("окно 5×5 пусто");
                let big = plate_query_window(point, s, &p, 4).expect("окно 9×9 пусто");
                assert_eq!(small.plate_id(), big.plate_id(), "точка ({x}, {z})");
                assert_eq!(small.b.map(|b| b.id), big.b.map(|b| b.id));
                assert!(
                    (small.nearest_boundary_l() - big.nearest_boundary_l()).abs() < 1e-9,
                    "расстояние до границы"
                );
            }
        }
    }

    #[test]
    fn boundary_type_is_symmetric() {
        let p = params();
        let sample = plate_query(Vec2::new(13.0, -7.0), 42, &p).unwrap();
        let (a, b) = (sample.a, sample.b.unwrap());
        let one = boundary_type(&a, &b, sample.q, sample.n_ab, &p);
        let two = boundary_type(&b, &a, sample.q, sample.n_ab.scale(-1.0), &p);
        assert!((one.conv - two.conv).abs() < 1e-12);
        assert!((one.div - two.div).abs() < 1e-12);
        assert!((one.tr - two.tr).abs() < 1e-12);
    }

    #[test]
    fn continent_field_sign_matches_center() {
        let p = params();
        let sample = plate_query(Vec2::new(0.0, 0.0), 7, &p).unwrap();
        let kappa = continent_field(&sample.a, sample.a.center);
        if sample.a.has_continent {
            assert!(kappa > 0.0, "в центре материка κ = {kappa}");
        } else {
            assert!(kappa < 0.0);
        }
        assert_eq!(continent_fraction(-6.0), 0.0);
        assert_eq!(continent_fraction(6.0), 1.0);
    }

    #[test]
    fn plate_colors_are_not_all_the_same() {
        let mut colors = std::collections::BTreeSet::new();
        for k in 0..64_u64 {
            colors.insert(super::plate_color(k * 0x9E37_79B9_7F4A_7C15));
        }
        assert!(colors.len() > 50, "уникальных цветов: {}", colors.len());
        let mut channel_varies = false;
        for color in &colors {
            if color[1] != color[0] || color[2] != color[0] {
                channel_varies = true;
            }
        }
        assert!(channel_varies, "все каналы цвета одинаковы");
    }

    #[test]
    fn warp_does_not_fold_the_plane() {
        let p = params();
        for k in 0..200 {
            let x = unit(splitmix64(k)) * 200.0 - 100.0;
            let z = unit(splitmix64(k + 999)) * 200.0 - 100.0;
            let det = warp_jacobian(Vec2::new(x, z), 42, &p);
            // Порог 0,2 из 7.3.2 проверяется слоем `jacobian` в песочнице;
            // здесь проверяем более слабое свойство: отображение не складывается.
            assert!(det > 0.0, "якобиан = {det} в ({x}, {z})");
        }
    }
}
