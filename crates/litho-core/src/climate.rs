//! Климат: полюса-точки, температура, ветер, влажность и осадки (план 7.5–7.7,
//! задачи T30–T33). Все формулы детерминированы: только `f64`, свои `dexp`/`dpow`
//! и шумы из `noise.rs`, без системной математики.

use crate::hash::{splitmix64, unit};
use crate::math::{Vec2, clamp, clamp01, dexp, dpow, lerp, piecewise, smoothstep};
use crate::noise::{fbm2, warp2};
use crate::params::Params;
use crate::seed::{
    SALT_PHI_NOISE, SALT_POLES_SEED, SALT_POLES_WARP_X, SALT_POLES_WARP_Z, cell_hash, subseed,
};

/// Что нужно климату от слоя коры в произвольной macro-точке.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceSample {
    pub e_km: f64,
    pub iota_l: f64,
    pub mountain: f64,
    pub plateau: f64,
    /// Веса k1…k7 у ближайшей границы и расстояние до неё (нужны рекам T50).
    pub boundary_weights: [f64; 7],
    pub sigma_l: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoleSample {
    /// Псевдоширота 0…90°: 0 на «экваторе», 90 у полюса.
    pub phi_deg: f64,
    /// Направление роста φ («на север»), единичный вектор.
    pub north: Vec2,
    /// «Восток»: поворот `north` на 90° по часовой стрелке на карте.
    pub east: Vec2,
    pub pole: Vec2,
    pub distance_l: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TemperatureSample {
    pub t_c: f64,
    pub sst_c: f64,
    pub amp_c: f64,
    pub t_bio: f64,
    pub t_min: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrecipSample {
    /// Влагосодержание воздуха μ в точке.
    pub mu: f64,
    /// Континентальность c ∈ [0, 1].
    pub cont: f64,
    pub p_mm: f64,
    pub pet_mm: f64,
    pub ratio: f64,
    pub slope: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClimateSample {
    pub point: Vec2,
    pub surface: SurfaceSample,
    pub pole: PoleSample,
    pub wind: Vec2,
    pub temperature: TemperatureSample,
    pub precip: PrecipSample,
}

/// Ровно один полюс в каждой ячейке сетки `G_P`, со случайным отступом.
fn pole_in_cell(seed: u64, i: i64, j: i64, params: &Params) -> Vec2 {
    let g = params.poles.cell_g_p_l;
    let inset = params.poles.inset_fraction;
    let h = cell_hash(subseed(seed, SALT_POLES_SEED), i, j, 0);
    let h2 = splitmix64(h);
    let fx = inset + (1.0 - 2.0 * inset) * unit(h);
    let fz = inset + (1.0 - 2.0 * inset) * unit(h2);
    Vec2::new((i as f64 + fx) * g, (j as f64 + fz) * g)
}

/// Ближайший и второй по близости полюс, псевдоширота и направления (7.5.1).
pub fn pole_sample(point: Vec2, seed: u64, params: &Params) -> Option<PoleSample> {
    let pol = &params.poles;
    let shifted = warp2(
        subseed(seed, SALT_POLES_WARP_X),
        subseed(seed, SALT_POLES_WARP_Z),
        point,
        pol.warp_lambda0_l,
        pol.warp_amplitude_l,
        pol.warp_octaves,
        pol.warp_gain,
    );
    let q = point.add(shifted);
    let g = pol.cell_g_p_l;
    let ci = (q.x / g).floor() as i64;
    let cj = (q.z / g).floor() as i64;
    let radius = pol.query_radius_cells;
    let mut best = (f64::INFINITY, Vec2::default());
    let mut second = (f64::INFINITY, Vec2::default());
    for di in -radius..=radius {
        for dj in -radius..=radius {
            let candidate = pole_in_cell(seed, ci + di, cj + dj, params);
            let d = q.sub(candidate).len();
            if d < best.0 {
                second = best;
                best = (d, candidate);
            } else if d < second.0 {
                second = (d, candidate);
            }
        }
    }
    if !best.0.is_finite() || !second.0.is_finite() {
        return None;
    }
    let f1 = best.0;
    let f2 = second.0;
    let total = f1 + f2;
    let t = if total > 0.0 { f1 / total } else { 0.0 };
    let phi0 = 90.0 * (1.0 - 2.0 * t);
    // Д-021: дрожь псевдошироты стала многооктавной и настраиваемой —
    // границы холода рваные, а не концентрические окружности.
    let jitter = fbm2(
        subseed(seed, SALT_PHI_NOISE),
        point.x,
        point.z,
        pol.phi_noise_lambda_l,
        pol.phi_noise_octaves,
        pol.phi_noise_gain,
    );
    let phi_deg = clamp(phi0 + pol.phi_noise_degrees * jitter, 0.0, 90.0);

    // ∇t с точностью до положительного множителя; «север» — против него.
    let u1 = q.sub(best.1).scale(1.0 / f1.max(1e-9));
    let u2 = q.sub(second.1).scale(1.0 / f2.max(1e-9));
    let grad = u1.scale(f2).sub(u2.scale(f1));
    let north = if grad.len() > 1e-9 {
        grad.scale(-1.0).normalized()
    } else {
        Vec2::new(0.0, -1.0)
    };
    Some(PoleSample {
        phi_deg,
        north,
        east: north.perp(),
        pole: best.1,
        distance_l: f1,
    })
}

/// Среднегодовая температура на уровне моря: `28 − 52·(φ/90)^1.6`.
pub fn sea_level_temp(phi_deg: f64, params: &Params) -> f64 {
    let c = &params.climate;
    let lat = clamp(phi_deg, 0.0, 90.0) / 90.0;
    c.t_equator_c - c.t_pole_drop_c * dpow(lat, c.t_lat_exponent)
}

/// Температура с учётом высоты, континентальности и аналитических сезонов (7.5.3).
pub fn temperature(phi_deg: f64, e_km: f64, cont: f64, params: &Params) -> TemperatureSample {
    let c = &params.climate;
    let t_sl = sea_level_temp(phi_deg, params);
    let cont = clamp01(cont);
    let delta_c = cont
        * (c.cont_tropical_c * (1.0 - smoothstep(25.0, 45.0, phi_deg))
            - c.cont_polar_c * smoothstep(40.0, 70.0, phi_deg));
    let t_c = t_sl + delta_c - c.lapse_c_per_km * e_km.max(0.0);
    let amp_lat = c.amp_base_c + c.amp_lat_c * smoothstep(5.0, 75.0, phi_deg);
    let amp_c = amp_lat * (c.amp_sea_factor + c.amp_land_factor * cont);
    let (t_bio, t_min) = seasonal(t_c, amp_c, params);
    TemperatureSample {
        t_c,
        sst_c: t_sl.max(c.sst_min_c),
        amp_c,
        t_bio,
        t_min,
    }
}

/// cos(2π(m + 0,5)/12), каждое значение дважды. Константы, а не системный cos.
pub const MONTH_COS: [f64; 12] = [
    0.965_925_8,
    std::f64::consts::FRAC_1_SQRT_2,
    0.258_819,
    -0.258_819,
    -std::f64::consts::FRAC_1_SQRT_2,
    -0.965_925_8,
    -0.965_925_8,
    -std::f64::consts::FRAC_1_SQRT_2,
    -0.258_819,
    0.258_819,
    std::f64::consts::FRAC_1_SQRT_2,
    0.965_925_8,
];

/// Биотемпература и самый холодный месяц при уже известных средней температуре
/// и сезонной амплитуде (быстрый путь T55 считает их поблочно).
pub fn seasonal(t_c: f64, amp_c: f64, params: &Params) -> (f64, f64) {
    let c = &params.climate;
    let mut bio = 0.0;
    for cos_m in MONTH_COS {
        bio += clamp(t_c + amp_c * cos_m, c.bio_min_c, c.bio_max_c);
    }
    (bio / 12.0, t_c - c.coldest_cos * amp_c)
}

/// Три пояса постоянных ветров (7.6). Вектор НЕ нормируется: в переходных
/// зонах направления гасят друг друга, как и задумано.
pub fn wind(phi_deg: f64, north: Vec2, east: Vec2, params: &Params) -> Vec2 {
    let c = &params.climate;
    let trade = north
        .scale(c.trade_n)
        .add(east.scale(c.trade_e))
        .normalized();
    let west = north.scale(c.west_n).add(east.scale(c.west_e)).normalized();
    let polar = north
        .scale(c.polar_n)
        .add(east.scale(c.polar_e))
        .normalized();
    let w_t = 1.0 - smoothstep(c.belt_trade_deg[0], c.belt_trade_deg[1], phi_deg);
    let rise_w = smoothstep(c.belt_west_deg[0], c.belt_west_deg[1], phi_deg);
    let fall_w = 1.0 - smoothstep(c.belt_polar_deg[0], c.belt_polar_deg[1], phi_deg);
    let w_w = rise_w * fall_w;
    let w_p = smoothstep(c.belt_polar_deg[0], c.belt_polar_deg[1], phi_deg);
    let strength = piecewise(
        &params.wind.strength_phi_deg,
        &params.wind.strength_s,
        phi_deg,
    );
    trade
        .scale(w_t)
        .add(west.scale(w_w))
        .add(polar.scale(w_p))
        .scale(strength)
}

fn fallback_dir(params: &Params) -> Vec2 {
    let c = &params.climate;
    Vec2::new(c.wind_fallback_dir[0], c.wind_fallback_dir[1]).normalized()
}

/// Ветер и псевдоширота в точке линии тока.
fn wind_at(point: Vec2, seed: u64, params: &Params) -> (Vec2, f64) {
    match pole_sample(point, seed, params) {
        Some(pol) => (wind(pol.phi_deg, pol.north, pol.east, params), pol.phi_deg),
        None => (fallback_dir(params), 45.0),
    }
}

fn w_sat(phi_deg: f64, params: &Params) -> f64 {
    let c = &params.climate;
    clamp(
        c.w_sat_base + c.w_sat_per_c * sea_level_temp(phi_deg, params),
        c.w_sat_min,
        c.w_sat_max,
    )
}

/// Осадки по поясу и переносу (7.7.4) на прямых входах — контрольные точки
/// 7.7.6 и тест зонального профиля считаются именно через неё.
pub fn precipitation(phi_deg: f64, mu: f64, wind_len: f64, slope: f64, params: &Params) -> f64 {
    let pr = &params.precip;
    let c = &params.climate;
    let itcz = phi_deg / pr.itcz_width_deg;
    let front = (phi_deg - pr.front_center_deg) / pr.front_width_deg;
    let p_c = pr.itcz_precip_mm * dexp(-itcz * itcz) + pr.front_precip_mm * dexp(-front * front);
    let equator = phi_deg / c.local_fraction_width_deg;
    let a = pr.local_fraction_base + c.local_fraction_equator * dexp(-equator * equator);
    let u = c.uplift_base + c.uplift_per_slope * slope.min(c.uplift_slope_max);
    p_c * (a + (1.0 - a) * mu) + pr.advection_mm * wind_len.max(c.advection_wind_min) * mu * u
}

/// Испаряемость и индекс сухости Холдриджа (7.7.5).
pub fn dryness(t_bio: f64, p_mm: f64, params: &Params) -> (f64, f64) {
    let c = &params.climate;
    let pet = c.pet_per_bio_c * t_bio.max(0.0);
    (pet, pet / p_mm.max(c.p_min_mm))
}

struct RayResult {
    mu: f64,
    cont_ray: f64,
    slope: f64,
}

/// Линия тока против ветра (7.7.1–7.7.3): 32 шага по 0,25 L, влага считается
/// от дальнего конца к точке, заодно собирается континентальность.
fn trace_ray<F>(point: Vec2, seed: u64, params: &Params, field: &F, wind_dir: Vec2) -> RayResult
where
    F: Fn(Vec2) -> SurfaceSample,
{
    let pr = &params.precip;
    let c = &params.climate;
    let steps = pr.ray_steps;
    let ds = pr.ray_step_l;
    let mut points = Vec::with_capacity(steps + 1);
    let mut phis = Vec::with_capacity(steps + 1);
    points.push(point);
    let mut prev = fallback_dir(params);
    let mut q = point;
    for _ in 0..steps {
        let (v, phi) = wind_at(q, seed, params);
        phis.push(phi);
        let dir = if v.len() < 0.05 { prev } else { v.normalized() };
        q = q.sub(dir.scale(ds));
        points.push(q);
        prev = dir;
    }
    let (_, phi_last) = wind_at(q, seed, params);
    phis.push(phi_last);
    let mut heights = Vec::with_capacity(points.len());
    for candidate in &points {
        heights.push(field(*candidate).e_km);
    }

    let n = steps;
    let sat_last = w_sat(phis[n], params);
    let mut moisture = if heights[n] < 0.0 {
        sat_last
    } else {
        c.w_land_start * sat_last
    };
    for k in (0..n).rev() {
        if heights[k] < 0.0 {
            let sat = w_sat(phis[k], params);
            moisture += (sat - moisture) * (1.0 - dexp(-ds / pr.ocean_length_l));
        } else {
            moisture *= dexp(-ds / pr.land_length_l);
            let climb = (heights[k] - heights[k + 1]).max(0.0);
            moisture *= dexp(-climb / pr.orographic_height_km);
        }
    }

    let mut sea_weight = 0.0;
    let mut total_weight = 0.0;
    for (k, height) in heights.iter().enumerate() {
        let weight = dexp(-(k as f64) * ds / c.cont_decay_l);
        total_weight += weight;
        if *height < 0.0 {
            sea_weight += weight;
        }
    }
    let m_ind = if total_weight > 0.0 {
        sea_weight / total_weight
    } else {
        0.0
    };
    let upwind = point.sub(wind_dir.scale(c.slope_probe_l));
    let slope = ((heights[0] - field(upwind).e_km) / c.slope_probe_l).max(0.0);
    RayResult {
        mu: moisture,
        cont_ray: 1.0 - m_ind,
        slope,
    }
}

/// Полный климат в точке (Э1d).
pub fn sample<F>(point: Vec2, seed: u64, params: &Params, field: &F) -> Option<ClimateSample>
where
    F: Fn(Vec2) -> SurfaceSample,
{
    let c = &params.climate;
    let surface = field(point);
    let pole = pole_sample(point, seed, params)?;
    let wind_vec = wind(pole.phi_deg, pole.north, pole.east, params);
    let wind_len = wind_vec.len();
    let dir = if wind_len > 1e-9 {
        wind_vec.normalized()
    } else {
        fallback_dir(params)
    };
    let ray = trace_ray(point, seed, params, field, dir);
    let c_iso = smoothstep(0.0, c.c_iso_end_l, surface.iota_l);
    let cont = lerp(
        c_iso,
        ray.cont_ray,
        smoothstep(c.c_ray_wind_lo, c.c_ray_wind_hi, wind_len),
    );
    let temperature = temperature(pole.phi_deg, surface.e_km, cont, params);
    let p_mm = precipitation(pole.phi_deg, ray.mu, wind_len, ray.slope, params);
    let (pet_mm, ratio) = dryness(temperature.t_bio, p_mm, params);
    Some(ClimateSample {
        point,
        surface,
        pole,
        wind: wind_vec,
        temperature,
        precip: PrecipSample {
            mu: ray.mu,
            cont,
            p_mm,
            pet_mm,
            ratio,
            slope: ray.slope,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::{
        PoleSample, dryness, pole_sample, precipitation, sample, sea_level_temp, temperature, wind,
    };
    use crate::classify::classify_inputs;
    use crate::math::Vec2;
    use crate::mode::Mode;
    use crate::params::Params;
    use crate::version::GenVersion;

    fn params() -> Params {
        Params::defaults(GenVersion::V0, Mode::Gameplay)
    }

    /// Контрольная точка 7.7.6: прямые входы, без карты и без дрожания.
    fn point(
        phi: f64,
        e: f64,
        cont: f64,
        mu: f64,
        v: f64,
        slope: f64,
        p: &Params,
    ) -> (f64, f64, f64, f64) {
        let t = temperature(phi, e, cont, p);
        let precip = precipitation(phi, mu, v, slope, p);
        let (_, ratio) = dryness(t.t_bio, precip, p);
        (t.t_bio, precip, ratio, t.t_min)
    }

    fn near(actual: f64, expected: f64) {
        let span = expected.abs().max(1.0) * 0.05;
        assert!(
            (actual - expected).abs() <= span,
            "ожидалось {expected} ±5 %, получено {actual}"
        );
    }

    #[test]
    fn sea_level_temperature_matches_plan_table() {
        let p = params();
        for (phi, expected) in [
            (0.0, 28.0),
            (30.0, 19.0),
            (45.0, 10.8),
            (60.0, 0.8),
            (75.0, -10.8),
            (90.0, -24.0),
        ] {
            assert!(
                (sea_level_temp(phi, &p) - expected).abs() < 0.6,
                "φ={phi}: {} вместо {expected}",
                sea_level_temp(phi, &p)
            );
        }
    }

    #[test]
    #[allow(clippy::type_complexity)] // таблица контрольных точек 7.7.6
    fn control_points_a_to_f_match_plan_7_7_6() {
        let p = params();
        let cases: [(&str, (f64, f64, f64, f64, f64, f64), (f64, f64, f64), &str); 7] = [
            (
                "A",
                (3.0, 0.05, 0.1, 1.25, 0.28, 0.0),
                (27.7, 2580.0, 0.63),
                "jungle",
            ),
            (
                "B",
                (3.0, 0.3, 0.8, 0.46, 0.28, 0.0),
                (27.4, 1700.0, 0.95),
                "jungle",
            ),
            (
                "C",
                (25.0, 0.4, 1.0, 0.14, 0.53, 0.0),
                (20.7, 85.0, 14.0),
                "desert",
            ),
            (
                "D",
                (47.0, 0.1, 0.05, 0.636, 0.92, 0.0),
                (8.9, 1120.0, 0.47),
                "dark_forest",
            ),
            (
                "D′",
                (47.0, 0.1, 0.05, 0.636, 0.92, 2.0),
                (8.9, 1990.0, 0.26),
                "old_growth_spruce_taiga",
            ),
            (
                "E",
                (50.0, 0.3, 1.0, 0.074, 1.0, 0.0),
                (7.9, 260.0, 1.8),
                "plains",
            ),
            (
                "F",
                (60.0, 0.2, 0.8, 0.25, 0.4, 0.0),
                (4.0, 315.0, 0.75),
                "snowy_taiga",
            ),
        ];
        for (name, inputs, expected, biome) in cases {
            let (t_bio, p_mm, ratio, t_min) = point(
                inputs.0, inputs.1, inputs.2, inputs.3, inputs.4, inputs.5, &p,
            );
            near(t_bio, expected.0);
            near(p_mm, expected.1);
            near(ratio, expected.2);
            let class = classify_inputs(t_bio, ratio, t_min, &p);
            assert_eq!(class.cell.biome, biome, "точка {name}: биом");
        }
    }

    #[test]
    fn zonal_precipitation_has_two_maxima_and_a_subtropical_minimum() {
        let p = params();
        // Конвекционная часть профиля: без переноса (μ = 0), иначе ровная
        // добавка адвекции сгладила бы субтропический минимум.
        let equator = precipitation(0.0, 0.0, 0.6, 0.0, &p);
        let subtropics = precipitation(25.0, 0.0, 0.6, 0.0, &p);
        let front = precipitation(50.0, 0.0, 0.6, 0.0, &p);
        assert!(equator > subtropics * 10.0, "экватор слишком сухой");
        assert!(front > subtropics * 10.0, "полярный фронт слишком сухой");
    }

    #[test]
    fn poles_form_a_regular_non_square_lattice() {
        let p = params();
        let mut min_phi = 90.0_f64;
        let mut max_phi = 0.0_f64;
        for k in 0..1600 {
            let x = (k % 40) as f64 * 10.0 - 200.0;
            let z = (k / 40) as f64 * 10.0 - 200.0;
            let PoleSample {
                phi_deg,
                north,
                east,
                distance_l,
                ..
            } = pole_sample(Vec2::new(x, z), 42, &p).unwrap();
            assert!((0.0..=90.0).contains(&phi_deg));
            assert!(distance_l < 1.2 * p.poles.cell_g_p_l);
            assert!((north.len() - 1.0).abs() < 1e-9);
            assert!((east.len() - 1.0).abs() < 1e-9);
            assert!(north.dot(east).abs() < 1e-9);
            min_phi = min_phi.min(phi_deg);
            max_phi = max_phi.max(phi_deg);
        }
        assert!(min_phi < 20.0 && max_phi > 70.0, "пояса не покрывают 0…90°");
    }

    #[test]
    fn wind_is_bounded_and_weak_at_equator() {
        let p = params();
        let north = Vec2::new(0.0, -1.0);
        let east = Vec2::new(1.0, 0.0);
        for phi in [0.0, 15.0, 30.0, 45.0, 60.0, 75.0, 90.0] {
            let v = wind(phi, north, east, &p);
            assert!(v.len() <= 1.0 + 1e-9, "φ={phi}: |V|={}", v.len());
        }
        assert!(wind(0.0, north, east, &p).len() < 0.4);
    }

    #[test]
    fn climate_sample_is_finite_on_flat_and_mountainous_surfaces() {
        let p = params();
        for height in [-4.5_f64, 0.0, 0.6, 3.0] {
            let field = |point: Vec2| crate::climate::SurfaceSample {
                e_km: height,
                iota_l: point.x.abs() * 0.1,
                mountain: 0.2,
                plateau: 0.0,
                boundary_weights: [0.0; 7],
                sigma_l: 0.0,
            };
            let s = sample(Vec2::new(3.0, -7.0), 42, &p, &field).unwrap();
            assert!(s.temperature.t_bio.is_finite());
            assert!(s.precip.p_mm.is_finite() && s.precip.p_mm >= 0.0);
            assert!(s.precip.ratio.is_finite() && s.precip.ratio > 0.0);
            assert!((0.0..=1.0).contains(&s.precip.cont));
        }
    }
}
