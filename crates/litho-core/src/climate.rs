//! Климат: полюса-точки, температура, ветер, влажность и осадки (план 7.5–7.7,
//! задачи T30–T33). Все формулы детерминированы: только `f64`, свои `dexp`/`dpow`
//! и шумы из `noise.rs`, без системной математики.

use crate::hash::{splitmix64, unit};
use crate::math::{
    Vec2, clamp, clamp01, datan2, dexp, dpow, lerp, piecewise, rotate_unit, smoothstep,
};
use crate::noise::{fbm2, fbm2_unit, warp2};
use crate::params::Params;
use crate::seed::{
    SALT_MOISTURE_BEND, SALT_PHI_NOISE, SALT_POLES_SEED, SALT_POLES_WARP_X, SALT_POLES_WARP_Z,
    cell_hash, subseed,
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

/// Геометрия двух ближайших полюсов после искажения домена. Выделена отдельно,
/// чтобы скаляр φ для конечных разностей T131 считался той же формулой, что и
/// φ в `pole_sample`, без второго расхождения реализаций.
fn pole_geometry(point: Vec2, seed: u64, params: &Params) -> Option<(Vec2, Vec2, f64, Vec2, f64)> {
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
    Some((q, best.1, best.0, second.1, second.0))
}

/// Скалярная псевдоширота 0…90° (7.5.1 + Д-021). Непрерывна при смене
/// второго полюса: расстояния f1/f2 входят симметрично через f1/(f1+f2).
fn phi_scalar(point: Vec2, seed: u64, params: &Params) -> Option<f64> {
    let (_, _, f1, _, f2) = pole_geometry(point, seed, params)?;
    Some(phi_from_distances(f1, f2, point, seed, params))
}

fn phi_from_distances(f1: f64, f2: f64, point: Vec2, seed: u64, params: &Params) -> f64 {
    let pol = &params.poles;
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
    let raw = phi0 + pol.phi_noise_degrees * jitter;
    if params.version.is_water_relief_dev() {
        soft_phi(raw, params.boundaries.wind_phi_soft_deg)
    } else {
        clamp(raw, 0.0, 90.0)
    }
}

/// T131: C1-насыщение φ у 0 и 90. Жёсткий clamp создавал плато с нулевым
/// градиентом и разворот «севера» на его границе; экспоненциальный хвост
/// сохраняет непрерывную производную и не даёт φ выйти за диапазон.
fn soft_phi(x: f64, margin: f64) -> f64 {
    if margin <= 0.0 {
        return clamp(x, 0.0, 90.0);
    }
    let lo = if x < margin {
        margin * dexp((x - margin) / margin)
    } else {
        x
    };
    if lo > 90.0 - margin {
        90.0 - margin * dexp((90.0 - margin - lo) / margin)
    } else {
        lo
    }
}

/// Ближайший и второй по близости полюс, псевдоширота и направления (7.5.1).
pub fn pole_sample(point: Vec2, seed: u64, params: &Params) -> Option<PoleSample> {
    let (q, best_pole, f1, second_pole, f2) = pole_geometry(point, seed, params)?;
    let phi_deg = phi_from_distances(f1, f2, point, seed, params);
    let north = if params.version.is_water_relief_dev() {
        // T131 / Б3: «север» — минус градиент скалярной φ центральными
        // разностями. Шаг 0.25–0.5 L сглаживает смену второго полюса; φ при
        // этом остаётся той же непрерывной функцией, что и температура.
        let h = params.boundaries.wind_fd_step_l;
        let dx = Vec2::new(h, 0.0);
        let dz = Vec2::new(0.0, h);
        let pxp = phi_scalar(point.add(dx), seed, params)?;
        let pxm = phi_scalar(point.sub(dx), seed, params)?;
        let pzp = phi_scalar(point.add(dz), seed, params)?;
        let pzm = phi_scalar(point.sub(dz), seed, params)?;
        let grad = Vec2::new((pxp - pxm) / (2.0 * h), (pzp - pzm) / (2.0 * h));
        wind_north(grad, f1, params)
    } else {
        // v0: прежний аналитический ∇t, побайтово тот же порядок операций.
        let u1 = q.sub(best_pole).scale(1.0 / f1.max(1e-9));
        let u2 = q.sub(second_pole).scale(1.0 / f2.max(1e-9));
        let grad = u1.scale(f2).sub(u2.scale(f1));
        if grad.len() > 1e-9 {
            grad.scale(-1.0).normalized()
        } else {
            Vec2::new(0.0, -1.0)
        }
    };
    Some(PoleSample {
        phi_deg,
        north,
        east: north.perp(),
        pole: best_pole,
        distance_l: f1,
    })
}

/// T131: непрерывный «север» из −∇φ. У полюса и на плато φ=0 градиент
/// вырождается; вместо скачка на запасное направление плавно поворачиваем
/// к нему. Порог по |∇φ| убирает дрожь на экваторе, радиус у полюса — сингулярность
/// радиального направления. Поворот идёт по дуге, не через нулевую сумму.
fn wind_north(grad: Vec2, distance_to_pole_l: f64, params: &Params) -> Vec2 {
    let fallback = fallback_dir(params);
    let g = grad.len();
    if g <= 1e-12 {
        return fallback;
    }
    let target = grad.scale(-1.0 / g);
    let b = &params.boundaries;
    let mut t = smoothstep(0.0, b.wind_grad_floor_deg_per_l, g)
        * smoothstep(0.0, b.wind_pole_fade_l, distance_to_pole_l);
    // Если target почти противоположен запасному направлению, кратчайшая дуга
    // меняет знак при переходе через 180°. Гасим вес там, чтобы не было разворота.
    t *= smoothstep(-0.98, -0.8, fallback.dot(target));
    blend_unit(fallback, target, t)
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
    let strength = piecewise(
        &params.wind.strength_phi_deg,
        &params.wind.strength_s,
        phi_deg,
    );
    if params.version.is_water_relief_dev() {
        // T131: векторная сумма противоположных пассата и западных ветров
        // проходила через ноль и разворачивала направление на 180° за блок.
        // В v1-dev поворачиваем само направление по дуге, а силу оставляем
        // прежней piecewise-функцией φ. У экватора/переходов нет нулевого вектора.
        // v1-dev расширяет переходы поясов: исходные 10° давали до 15°/блок
        // при |∇φ|≈5 °/L. Центры поясов прежние, полуширина настраиваемая.
        let trade_center = 0.5 * (c.belt_trade_deg[0] + c.belt_trade_deg[1]);
        let polar_center = 0.5 * (c.belt_polar_deg[0] + c.belt_polar_deg[1]);
        let fade = params.boundaries.wind_belt_fade_deg;
        let tw = smoothstep(trade_center - fade, trade_center + fade, phi_deg);
        let wp = smoothstep(polar_center - fade, polar_center + fade, phi_deg);
        let mid = blend_unit(trade, west, tw);
        return blend_unit(mid, polar, wp).scale(strength);
    }
    let w_t = 1.0 - smoothstep(c.belt_trade_deg[0], c.belt_trade_deg[1], phi_deg);
    let rise_w = smoothstep(c.belt_west_deg[0], c.belt_west_deg[1], phi_deg);
    let fall_w = 1.0 - smoothstep(c.belt_polar_deg[0], c.belt_polar_deg[1], phi_deg);
    let w_w = rise_w * fall_w;
    let w_p = smoothstep(c.belt_polar_deg[0], c.belt_polar_deg[1], phi_deg);
    trade
        .scale(w_t)
        .add(west.scale(w_w))
        .add(polar.scale(w_p))
        .scale(strength)
}

/// Кратчайший поворот единичного направления a к b доля t ∈ [0,1].
/// Используется только v1-dev; при почти противоположных векторах дуга
/// выбирается знаком det, а не нормализацией вырожденной суммы.
fn blend_unit(a: Vec2, b: Vec2, t: f64) -> Vec2 {
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let cross = a.x * b.z - a.z * b.x;
    let dot = a.dot(b);
    rotate_unit(a, t * datan2(cross, dot))
}

pub fn fallback_dir(params: &Params) -> Vec2 {
    let c = &params.climate;
    Vec2::new(c.wind_fallback_dir[0], c.wind_fallback_dir[1]).normalized()
}

/// Ветер и псевдоширота в точке линии тока.
pub fn wind_at(point: Vec2, seed: u64, params: &Params) -> (Vec2, f64) {
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

/// T130: поворот против ветра гладким полем мировых координат. Поле не
/// зависит от номера шага/истории луча, поэтому соседние лучи не расходятся
/// из-за накопленной ошибки. Малый угол считается рядом, затем вектор
/// нормируется; системных sin/cos нет.
fn bend_ray(dir: Vec2, point: Vec2, seed: u64, params: &Params) -> Vec2 {
    if !params.version.is_water_relief_dev() {
        return dir;
    }
    let b = &params.boundaries;
    let n = fbm2_unit(
        subseed(seed, SALT_MOISTURE_BEND),
        point.x,
        point.z,
        b.ray_bend_lambda_l,
        b.ray_bend_octaves,
        b.ray_bend_gain,
    );
    let degrees = clamp(
        n * b.ray_bend_sigma_deg,
        -b.ray_bend_max_deg,
        b.ray_bend_max_deg,
    );
    let a = degrees * (std::f64::consts::PI / 180.0);
    let a2 = a * a;
    let sin_a = a * (1.0 - a2 / 6.0 * (1.0 - a2 / 20.0));
    let cos_a = 1.0 - a2 / 2.0 * (1.0 - a2 / 12.0 * (1.0 - a2 / 30.0));
    Vec2::new(dir.x * cos_a - dir.z * sin_a, dir.x * sin_a + dir.z * cos_a).normalized()
}

#[derive(Clone, Copy, Debug)]
pub struct RayResult {
    pub mu: f64,
    pub cont_ray: f64,
    pub slope: f64,
}

/// Линия тока против ветра (7.7.1–7.7.3): 32 шага по 0,25 L, влага считается
/// от дальнего конца к точке, заодно собирается континентальность.
///
/// Д-021: ветер берётся из `wind_fn`, а не из `pole_sample` напрямую. Эталон
/// подставляет точные полюса, быстрый путь — coarse-решётку ветра. Луч всегда
/// стартует из самой точки: результат 8-L луча при рябистом рельефе нельзя
/// интерполировать по узлам coarse-тайла в 5 L.
pub fn trace_ray<F, W>(
    point: Vec2,
    params: &Params,
    field: &F,
    wind_dir: Vec2,
    wind_fn: &W,
    seed: u64,
) -> RayResult
where
    F: Fn(Vec2) -> SurfaceSample,
    W: Fn(Vec2) -> (Vec2, f64),
{
    if params.version.is_water_relief_dev() && params.boundaries.ray_fan_rays > 1 {
        return trace_fan(point, params, field, wind_dir, wind_fn, seed);
    }
    trace_single_ray(point, params, field, wind_dir, wind_fn, seed)
}

/// T136 / Б2: гауссов веер линий тока против ветра. Усредняются μ,
/// континентальность луча и наветренный уклон: это одна и та же полутень
/// Smith–Barstad, а не три независимых климата. Порядок лучей фиксирован от
/// отрицательного угла к положительному, сумма весов нормируется один раз.
fn trace_fan<F, W>(
    point: Vec2,
    params: &Params,
    field: &F,
    wind_dir: Vec2,
    wind_fn: &W,
    seed: u64,
) -> RayResult
where
    F: Fn(Vec2) -> SurfaceSample,
    W: Fn(Vec2) -> (Vec2, f64),
{
    let b = &params.boundaries;
    let n = b.ray_fan_rays;
    let half = b.ray_fan_half_angle_deg;
    let sigma = b.ray_fan_sigma_deg;
    let mut mu = 0.0;
    let mut cont = 0.0;
    let mut slope = 0.0;
    let mut total = 0.0;
    for k in 0..n {
        let t = k as f64 / (n - 1) as f64 * 2.0 - 1.0;
        let offset_deg = t * half;
        let weight = dexp(-0.5 * (offset_deg / sigma) * (offset_deg / sigma));
        let angle = offset_deg * (std::f64::consts::PI / 180.0);
        // Поворачиваем сам ветер внутри луча, а не только slope_probe:
        // иначе веер менял бы уклон, но все линии тока шли бы по одному пути.
        let fan_wind = |q: Vec2| {
            let (v, phi) = wind_fn(q);
            (rotate_unit(v, angle).scale(v.len()), phi)
        };
        let ray = trace_single_ray(
            point,
            params,
            field,
            rotate_unit(wind_dir, angle),
            &fan_wind,
            seed,
        );
        mu += weight * ray.mu;
        cont += weight * ray.cont_ray;
        slope += weight * ray.slope;
        total += weight;
    }
    RayResult {
        mu: mu / total,
        cont_ray: cont / total,
        slope: slope / total,
    }
}

fn trace_single_ray<F, W>(
    point: Vec2,
    params: &Params,
    field: &F,
    wind_dir: Vec2,
    wind_fn: &W,
    seed: u64,
) -> RayResult
where
    F: Fn(Vec2) -> SurfaceSample,
    W: Fn(Vec2) -> (Vec2, f64),
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
        let (v, phi) = wind_fn(q);
        phis.push(phi);
        let dir = if v.len() < 0.05 { prev } else { v.normalized() };
        let dir = bend_ray(dir, q, seed, params);
        q = q.sub(dir.scale(ds));
        points.push(q);
        prev = dir;
    }
    let (_, phi_last) = wind_fn(q);
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
    let ray = trace_ray(
        point,
        params,
        field,
        dir,
        &|q| wind_at(q, seed, params),
        seed,
    );
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
        PoleSample, bend_ray, dryness, pole_sample, precipitation, sample, sea_level_temp,
        temperature, trace_ray, wind,
    };
    use crate::classify::classify_inputs;
    use crate::climate::SurfaceSample;
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
    fn t130_ray_bend_is_smooth_bounded_and_v0_is_untouched() {
        let mut p = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        p.boundaries.validate().unwrap();
        let dir = Vec2::new(1.0, 0.0);
        let limit = p.boundaries.ray_bend_max_deg.to_radians().cos();
        let mut min_dot = 1.0_f64;
        let mut max_step = 0.0_f64;
        for k in 0..2000 {
            let q = Vec2::new(k as f64 * 0.013, k as f64 * -0.007);
            let a = bend_ray(dir, q, 42, &p);
            assert!((a.len() - 1.0).abs() < 1e-12);
            assert!(a.dot(dir) >= limit - 1e-12);
            min_dot = min_dot.min(a.dot(dir));
            let b = bend_ray(dir, q.add(Vec2::new(1.0 / 250.0, 0.0)), 42, &p);
            max_step = max_step.max(a.sub(b).len());
        }
        assert!(min_dot < 0.995, "луч остался прямым");
        assert!(max_step < 0.004, "скачок луча на блоке: {max_step}");
        let old = Params::defaults(GenVersion::V0, Mode::Gameplay);
        assert_eq!(bend_ray(dir, Vec2::new(3.0, -7.0), 42, &old), dir);
        let mut bad = p.clone();
        bad.boundaries.ray_bend_max_deg = 21.0;
        assert!(bad.boundaries.validate().is_err());
        bad = p.clone();
        bad.boundaries.ray_bend_lambda_l = 0.0;
        assert!(bad.boundaries.validate().is_err());
        p.boundaries.ray_bend_sigma_deg = 0.0;
        assert_eq!(bend_ray(dir, Vec2::new(3.0, -7.0), 42, &p), dir);
    }

    #[test]
    fn t130_bent_ray_breaks_a_straight_rain_shadow_without_a_block_jump() {
        let p = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        // Гладкий хребет поперёк ветра: прямоугольная стенка сама создавала бы
        // скачок при дискретном сэмплировании, которого нет в поле коры.
        let field = |q: Vec2| SurfaceSample {
            e_km: -1.0 + 4.0 * crate::math::dexp(-(q.x / 0.35) * (q.x / 0.35)),
            ..SurfaceSample::default()
        };
        let wind = |_| (Vec2::new(1.0, 0.0), 30.0);
        let ray = |z: f64, p: &Params| {
            trace_ray(Vec2::new(1.0, z), p, &field, Vec2::new(1.0, 0.0), &wind, 42).mu
        };
        let values: Vec<f64> = (-40..40).map(|k| ray(k as f64 * 0.1, &p)).collect();
        let spread = values.iter().copied().fold(f64::NEG_INFINITY, f64::max)
            - values.iter().copied().fold(f64::INFINITY, f64::min);
        // При σ=6° и хребте 3 км разброс μ около 0.007; прямой луч даёт 0.
        assert!(spread > 0.005, "край тени не изогнулся: {spread}");
        for pair in values.windows(2) {
            assert!((pair[0] - pair[1]).abs() < 0.1, "Δμ={}", pair[0] - pair[1]);
        }
        let old = Params::defaults(GenVersion::V0, Mode::Gameplay);
        let mut changed = old.clone();
        changed.boundaries.ray_bend_sigma_deg = 20.0;
        for z in [-3.0, 0.0, 3.0] {
            assert_eq!(ray(z, &old).to_bits(), ray(z, &changed).to_bits());
        }
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

#[cfg(test)]
mod t131_tests {
    use super::{fallback_dir, phi_scalar, pole_in_cell, pole_sample, wind, wind_north};
    use crate::hash::{splitmix64, unit};
    use crate::math::Vec2;
    use crate::mode::Mode;
    use crate::params::Params;
    use crate::version::GenVersion;

    /// T131 / §10.1: направление ветра между соседними окнами 32 блока
    /// меняется не больше чем на 5°. Скан включает случайные точки, оба режима,
    /// оба сиду и кольца вокруг полюсов, где ∇φ вырождается.
    #[test]
    fn wind_direction_changes_at_most_five_degrees_per_32_blocks() {
        for mode in Mode::ALL {
            let p = Params::defaults(GenVersion::V1Dev, mode);
            p.boundaries.validate().unwrap();
            let step = 32.0 / p.block.l_blocks;
            let mut worst = 0.0_f64;
            let mut check = |q: Vec2| {
                for d in [Vec2::new(step, 0.0), Vec2::new(0.0, step)] {
                    let a = pole_sample(q, 42, &p).unwrap();
                    let b = pole_sample(q.add(d), 42, &p).unwrap();
                    let wa = wind(a.phi_deg, a.north, a.east, &p).normalized();
                    let wb = wind(b.phi_deg, b.north, b.east, &p).normalized();
                    let deg = wa.dot(wb).clamp(-1.0, 1.0).acos().to_degrees();
                    worst = worst.max(deg);
                }
            };
            for k in 0..8000 {
                let q = Vec2::new(
                    unit(splitmix64(k)) * 512.0 - 256.0,
                    unit(splitmix64(k + 999_999)) * 512.0 - 256.0,
                );
                check(q);
            }
            for i in -2..=2_i64 {
                for j in -2..=2_i64 {
                    let pole = pole_in_cell(42, i, j, &p);
                    for r in [0.0, 0.01, 0.1, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0] {
                        for dir in [
                            Vec2::new(1.0, 0.0),
                            Vec2::new(0.0, 1.0),
                            Vec2::new(
                                -std::f64::consts::FRAC_1_SQRT_2,
                                std::f64::consts::FRAC_1_SQRT_2,
                            ),
                            Vec2::new(
                                std::f64::consts::FRAC_1_SQRT_2,
                                std::f64::consts::FRAC_1_SQRT_2,
                            ),
                        ] {
                            check(pole.add(dir.scale(r)));
                        }
                    }
                }
            }
            assert!(worst <= 5.0, "mode={mode:?}: скачок ветра {worst:.3}° > 5°");
        }
    }

    #[test]
    fn t131_north_is_unit_and_fallback_is_continuous_at_zero_gradient() {
        let p = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        let fb = fallback_dir(&p);
        assert_eq!(wind_north(Vec2::default(), 0.0, &p), fb);
        for g in [1e-9, 0.01, 0.1, 1.0, 5.0, 20.0, 50.0] {
            for dir in [
                Vec2::new(1.0, 0.0),
                Vec2::new(0.0, 1.0),
                Vec2::new(-0.6, 0.8),
            ] {
                let n = wind_north(dir.scale(g), 4.0, &p);
                assert!((n.len() - 1.0).abs() < 1e-12);
            }
        }
        // Малый градиент не должен разворачивать север скачком при переходе через ноль.
        let a = wind_north(Vec2::new(1e-6, 0.0), 40.0, &p);
        let b = wind_north(Vec2::new(-1e-6, 0.0), 40.0, &p);
        assert!(a.dot(b) > 0.999);
    }

    #[test]
    fn t131_phi_scalar_is_continuous_and_v0_pole_sample_is_bitwise_unchanged() {
        let dev = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        let old = Params::defaults(GenVersion::V0, Mode::Gameplay);
        for k in 0..2000 {
            let q = Vec2::new(k as f64 * 0.037 - 40.0, k as f64 * -0.021 + 15.0);
            let a = phi_scalar(q, 42, &dev).unwrap();
            let b = phi_scalar(q.add(Vec2::new(1.0 / 250.0, 0.0)), 42, &dev).unwrap();
            assert!((0.0..=90.0).contains(&a));
            assert!((a - b).abs() < 0.1, "Δφ={}", a - b);
            // v0: прежний аналитический градиент и жёсткий clamp, побитово.
            let s = pole_sample(q, 42, &old).unwrap();
            let (qq, best, f1, second, f2) = super::pole_geometry(q, 42, &old).unwrap();
            let u1 = qq.sub(best).scale(1.0 / f1.max(1e-9));
            let u2 = qq.sub(second).scale(1.0 / f2.max(1e-9));
            let grad = u1.scale(f2).sub(u2.scale(f1));
            let north = if grad.len() > 1e-9 {
                grad.scale(-1.0).normalized()
            } else {
                Vec2::new(0.0, -1.0)
            };
            assert_eq!(s.north.x.to_bits(), north.x.to_bits());
            assert_eq!(s.north.z.to_bits(), north.z.to_bits());
            assert_eq!(
                s.phi_deg.to_bits(),
                super::clamp(super::phi_from_distances(f1, f2, q, 42, &old), 0.0, 90.0).to_bits()
            );
        }
    }
}

#[cfg(test)]
mod t136_tests {
    use super::{RayResult, SurfaceSample, trace_ray};
    use crate::math::Vec2;
    use crate::mode::Mode;
    use crate::params::Params;
    use crate::version::GenVersion;

    fn fan_params() -> Params {
        Params::defaults(GenVersion::V1Dev, Mode::Gameplay)
    }

    /// Веер обязан менять сам путь линии тока, а не только slope_probe.
    /// Несимметричный хребет слева от ветра даёт разную μ у лучей веера.
    #[test]
    fn fan_averages_real_ray_paths_and_stays_bounded() {
        let p = fan_params();
        let field = |q: Vec2| SurfaceSample {
            e_km: -1.0
                + 4.0
                    * crate::math::dexp(-(q.x / 0.35) * (q.x / 0.35))
                    * crate::math::dexp(-((q.z + 1.0) / 0.6) * ((q.z + 1.0) / 0.6)),
            ..SurfaceSample::default()
        };
        let wind = |_| (Vec2::new(1.0, 0.0), 30.0);
        let point = Vec2::new(1.0, 0.0);
        let dir = Vec2::new(1.0, 0.0);
        let fan = trace_ray(point, &p, &field, dir, &wind, 42);
        let mut single = p.clone();
        single.boundaries.ray_fan_rays = 1;
        let one = trace_ray(point, &single, &field, dir, &wind, 42);
        assert!((fan.mu - one.mu).abs() > 1e-6, "веер не изменил линию тока");
        assert!(fan.mu.is_finite() && fan.mu >= 0.0);
        assert!((0.0..=1.0).contains(&fan.cont_ray));
        assert!(fan.slope.is_finite() && fan.slope >= 0.0);
        // Отключение веера возвращает ровно старый одиночный луч v1-dev.
        let again = trace_ray(point, &single, &field, dir, &wind, 42);
        assert_eq!(one.mu.to_bits(), again.mu.to_bits());
        assert_eq!(one.cont_ray.to_bits(), again.cont_ray.to_bits());
        assert_eq!(one.slope.to_bits(), again.slope.to_bits());
    }

    /// T160: три луча — явная квадратура −20°/0°/+20° с весами exp(−1/2),
    /// 1, exp(−1/2). Проверяем не только число лучей в параметрах, но и путь,
    /// вес и порядок накопления каждого из трёх результатов.
    #[test]
    fn default_fan_is_the_three_ray_weighted_sum() {
        let p = fan_params();
        assert_eq!(p.boundaries.ray_fan_rays, 3);
        let field = |q: Vec2| SurfaceSample {
            e_km: 0.2 * q.x - 0.1 * q.z + 2.0 * crate::math::dexp(-q.x * q.x),
            ..SurfaceSample::default()
        };
        let wind = |q: Vec2| (Vec2::new(1.0, 0.05 * q.z), 30.0);
        let point = Vec2::new(1.0, 0.3);
        let dir = Vec2::new(1.0, 0.0);
        let side = crate::math::dexp(-0.5);
        let total = 2.0 * side + 1.0;
        let mut expected = RayResult {
            mu: 0.0,
            cont_ray: 0.0,
            slope: 0.0,
        };
        for angle in [-20.0_f64, 0.0, 20.0] {
            let a = angle * (std::f64::consts::PI / 180.0);
            let rotated = crate::math::rotate_unit(dir, a);
            let fan_wind = |q: Vec2| {
                let (v, phi) = wind(q);
                (crate::math::rotate_unit(v, a).scale(v.len()), phi)
            };
            let ray = super::trace_single_ray(point, &p, &field, rotated, &fan_wind, 42);
            let weight = if angle == 0.0 { 1.0 } else { side };
            expected.mu += weight * ray.mu;
            expected.cont_ray += weight * ray.cont_ray;
            expected.slope += weight * ray.slope;
        }
        let actual = trace_ray(point, &p, &field, dir, &wind, 42);
        assert_eq!(actual.mu.to_bits(), (expected.mu / total).to_bits());
        assert_eq!(
            actual.cont_ray.to_bits(),
            (expected.cont_ray / total).to_bits()
        );
        assert_eq!(actual.slope.to_bits(), (expected.slope / total).to_bits());
    }

    /// На конечной полосе хребта веер сглаживает край тени поперёк ветра.
    #[test]
    fn fan_softens_the_edge_of_a_finite_rain_shadow() {
        let mut fan = fan_params();
        fan.boundaries.ray_fan_rays = 3;
        fan.boundaries.ray_fan_half_angle_deg = 20.0;
        fan.boundaries.ray_fan_sigma_deg = 20.0;
        let mut single = fan.clone();
        single.boundaries.ray_fan_rays = 1;
        let field = |q: Vec2| SurfaceSample {
            e_km: -1.0
                + 4.0
                    * crate::math::dexp(-(q.x / 0.35) * (q.x / 0.35))
                    * crate::math::dexp(-(q.z / 0.7) * (q.z / 0.7)),
            ..SurfaceSample::default()
        };
        let wind = |_| (Vec2::new(1.0, 0.0), 30.0);
        let max_step = |p: &Params| {
            let mut prev: Option<RayResult> = None;
            let mut worst = 0.0_f64;
            for k in -30..30 {
                let q = Vec2::new(1.0, k as f64 * 0.08);
                let r = trace_ray(q, p, &field, Vec2::new(1.0, 0.0), &wind, 42);
                if let Some(old) = prev {
                    worst = worst.max((r.mu - old.mu).abs());
                }
                prev = Some(r);
            }
            worst
        };
        let one = max_step(&single);
        let three = max_step(&fan);
        assert!(one > 0.0, "тест должен видеть край тени");
        assert!(three < one, "веер не сгладил край: {three} ≥ {one}");
    }

    #[test]
    fn v0_ignores_fan_parameters_bit_for_bit() {
        let old = Params::defaults(GenVersion::V0, Mode::Gameplay);
        let mut changed = old.clone();
        changed.boundaries.ray_fan_rays = 5;
        changed.boundaries.ray_fan_half_angle_deg = 20.0;
        changed.boundaries.ray_fan_sigma_deg = 1.0;
        let field = |q: Vec2| SurfaceSample {
            e_km: if q.x < 0.0 { -1.0 } else { 0.5 + 0.1 * q.z },
            ..SurfaceSample::default()
        };
        let wind = |q: Vec2| (Vec2::new(1.0, 0.1 * q.x), 30.0);
        for q in [Vec2::new(0.0, 0.0), Vec2::new(2.0, -3.0)] {
            let a = trace_ray(q, &old, &field, Vec2::new(1.0, 0.0), &wind, 42);
            let b = trace_ray(q, &changed, &field, Vec2::new(1.0, 0.0), &wind, 42);
            assert_eq!(a.mu.to_bits(), b.mu.to_bits());
            assert_eq!(a.cont_ray.to_bits(), b.cont_ray.to_bits());
            assert_eq!(a.slope.to_bits(), b.slope.to_bits());
        }
    }

    #[test]
    fn fan_parameters_validate() {
        let mut p = fan_params();
        p.boundaries.validate().unwrap();
        for rays in [0, 2, 4, 6] {
            let mut bad = p.clone();
            bad.boundaries.ray_fan_rays = rays;
            assert!(bad.boundaries.validate().is_err());
        }
        p.boundaries.ray_fan_half_angle_deg = 21.0;
        assert!(p.boundaries.validate().is_err());
        p = fan_params();
        p.boundaries.ray_fan_sigma_deg = 0.0;
        assert!(p.boundaries.validate().is_err());
    }
}
