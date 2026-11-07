//! Кора и макрорельеф (план 7.4, задачи T22–T24).
//!
//! Всё считается локально: поле материка `κ`, базовая высота коры, профили
//! границ U1–U7, горячие точки с цепочками, детализация и итоговые `e`, `M`,
//! `Pl`, `ι`. Высота в километрах условной планеты; в блоки игры её переведёт
//! ванильный генератор через параметры C/E/W (7.9, T41–T43).

use crate::hash::{hash3, splitmix64, unit};
use crate::math::{Vec2, clamp01, dexp, smoothstep};
use crate::noise::{fbm2, ridged2};
use crate::params::Params;
use crate::plates::{
    BoundaryType, Plate, PlateSample, boundary_type, boundary_weights, continent_field,
    continent_fraction, plate_query, seeds_in_cell,
};
use crate::seed::{
    CellRng, SALT_BOUNDARY_SIDE, SALT_CRUST_DETAIL, SALT_CRUST_KAPPA, SALT_HOTSPOT, cell_hash,
    subseed,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CrustSample {
    /// Смешанное поле материка, макро-единицы до берега.
    pub kappa_l: f64,
    /// Высота, км: −11 (жёлоб) … 0 (море) … +7 (пики).
    pub e_km: f64,
    /// «Горность» 0…1.
    pub mountain: f64,
    /// «Плато» 0…1.
    pub plateau: f64,
    /// Приближённое расстояние вглубь от берега, L.
    pub iota_l: f64,
    /// Веса k1…k7 у ближайших границ (для отладки и будущих слоёв).
    pub boundary_weights: [f64; 7],
    /// Доля материковой коры χ в точке (из того же κ, что и высота).
    pub chi: f64,
    /// Разложение высоты по физическим вкладам, км (Д-022).
    pub e_base_km: f64,
    pub boundary_u_km: f64,
    pub hotspot_km: f64,
    pub detail_km: f64,
    /// Какая плита «сверху» на ближайшей границе и знаковое расстояние σ.
    pub upper_is_a: bool,
    pub sigma_b_l: f64,
    /// Идентификаторы плит A/B/C и их χ из κ_plate (Д-012).
    pub plate_ids: [u64; 3],
    pub chi_plates: [f64; 3],
}

/// `exp(−(x/w)²)` на детерминированной экспоненте.
fn gauss(x: f64, width: f64) -> f64 {
    let t = x / width;
    dexp(-(t * t))
}

/// Поле материка одной плиты: эллипс плюс крупный шум береговой линии (7.4.1).
pub fn kappa_plate(plate: &Plate, q: Vec2, seed: u64, params: &Params) -> f64 {
    let c = &params.crust;
    if !plate.has_continent {
        return c.ocean_kappa_l;
    }
    let n = fbm2(
        subseed(seed, SALT_CRUST_KAPPA),
        q.x,
        q.z,
        c.kappa_noise_lambda_l,
        c.kappa_noise_octaves,
        c.kappa_noise_gain,
    );
    continent_field(plate, q) + c.kappa_noise_amplitude_l * n
}

/// Смешивание `κ` через границу плиты: на самом стыке получается среднее,
/// поэтому береговая линия не рвётся (7.4.1).
pub fn kappa_mix(sample: &PlateSample, seed: u64, params: &Params) -> f64 {
    let c = &params.crust;
    let ka = kappa_plate(&sample.a, sample.q, seed, params);
    let mut result = ka;
    if let Some(b) = sample.b {
        let kb = kappa_plate(&b, sample.q, seed, params);
        let blend = 1.0 - smoothstep(0.0, c.blend_width_l, sample.d_ab_l);
        result += c.boundary_mix_factor * sample.w_b * blend * (kb - ka);
    }
    if let Some(c_plate) = sample.c {
        let kc = kappa_plate(&c_plate, sample.q, seed, params);
        let blend = 1.0 - smoothstep(0.0, c.blend_width_l, sample.d_ac_l);
        result += c.boundary_mix_factor * sample.w_c * blend * (kc - result);
    }
    result
}

/// Поле материка как максимум вкладов ближайших плит с материком (Д-012).
///
/// В 7.4.1 `κ` привязано к ближайшей плите, поэтому материк обрезается
/// границами ячеек Вороного и мир выглядит как «один океан с крошками»
/// (12.6). Здесь берутся все центры материков в окне ячеек: континент может
/// лежать на нескольких плитах, как в реальности, а береговая линия больше не
/// повторяет прямолинейные швы. Вклад далёкой плиты ограничен уровнем океана,
/// поэтому смена окна не даёт разрывов на границах ячеек.
pub fn kappa_global(q: Vec2, seed: u64, params: &Params) -> f64 {
    let c = &params.crust;
    let g = params.plates.cell_g_l;
    let radius = c.continent_query_radius_cells;
    let ci = (q.x / g).floor() as i64;
    let cj = (q.z / g).floor() as i64;
    let noise = fbm2(
        subseed(seed, SALT_CRUST_KAPPA),
        q.x,
        q.z,
        c.kappa_noise_lambda_l,
        c.kappa_noise_octaves,
        c.kappa_noise_gain,
    );
    let mut result = c.ocean_kappa_l;
    for dj in -radius..=radius {
        for di in -radius..=radius {
            for plate in seeds_in_cell(seed, ci + di, cj + dj, params) {
                if !plate.has_continent {
                    continue;
                }
                let value = (continent_field(&plate, q) + c.kappa_noise_amplitude_l * noise)
                    .max(c.ocean_kappa_l);
                if value > result {
                    result = value;
                }
            }
        }
    }
    result
}

/// Базовая высота коры по полю материка (таблица 7.4.2).
pub fn e_base(kappa_l: f64, params: &Params) -> f64 {
    let c = &params.crust;
    if kappa_l >= 0.0 {
        return c.continent_height_km * (1.0 - dexp(-kappa_l / c.continent_scale_l));
    }
    if kappa_l >= -c.shelf_start_l {
        return c.shelf_depth_km * (-kappa_l / c.shelf_start_l);
    }
    if kappa_l >= -c.shelf_end_l {
        let t = smoothstep(0.0, 1.0, -kappa_l - c.shelf_start_l);
        return c.shelf_depth_km - c.slope_drop_km * t;
    }
    c.abyss_depth_km
}

/// Плато с одной стороны столкновения: `plat(σ)` из 7.4.3.
fn plateau_shape(sigma_l: f64, w_col_l: f64) -> f64 {
    smoothstep(-w_col_l, 0.5 * w_col_l, sigma_l)
        * (1.0 - smoothstep(1.2 * w_col_l, 3.5 * w_col_l, sigma_l))
}

/// Какая сторона границы «сверху»: у субдукции надвигается более молодая
/// плита, при равном возрасте сторону выбирает хэш пары (7.4.3, Д-022).
///
/// Это свойство **пары плит**, а не точки: раньше сюда подавались χ плит из
/// `kappa_plate`, и жёсткий порог `|χ_a − χ_n| > 0.25` перескакивал посреди
/// ровного склона. Знак `σ` переворачивался, `gauss(σ − d_island, w_island)`
/// включался/выключался целиком и давал скачок высоты на 1,12 км за один блок
/// (точка (336, 384) сида 42). Теперь ответ антисимметричен: при смене
/// ближайшей плиты A↔B он инвертируется ровно настолько, насколько `d_l`
/// меняет сторону, и `σ` остаётся непрерывным на самой границе.
fn upper_is_a(a: &Plate, neighbor: &Plate, seed: u64) -> bool {
    let low = a.id.min(neighbor.id);
    let high = a.id.max(neighbor.id);
    let hash_bit = hash3(
        subseed(seed, SALT_BOUNDARY_SIDE),
        low as i64,
        high as i64,
        0,
    ) & 1
        == 1;
    if (a.age - neighbor.age).abs() > 1e-12 {
        // Младшая (как правило, океаническая) плита сверху. Условие
        // антисимметрично: upper(A,B) == !upper(B,A).
        return a.age < neighbor.age;
    }
    if a.id < neighbor.id {
        hash_bit
    } else {
        !hash_bit
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct BoundaryContrib {
    u: f64,
    positive: f64,
    plateau: f64,
    k7_gauss: f64,
    sigma_l: f64,
    upper: bool,
}

impl BoundaryContrib {
    fn add(self, other: Self) -> Self {
        Self {
            u: self.u + other.u,
            positive: self.positive + other.positive,
            plateau: self.plateau + other.plateau,
            k7_gauss: self.k7_gauss + other.k7_gauss,
            sigma_l: self.sigma_l,
            upper: self.upper,
        }
    }
}

/// Вклад одной границы A|N: профили U1–U7, взвешенные k1…k7 (7.3.6, 7.4.3).
fn boundary_contrib(
    sample: &PlateSample,
    neighbor: &Plate,
    d_l: f64,
    normal: Vec2,
    weight: f64,
    seed: u64,
    params: &Params,
) -> BoundaryContrib {
    let c = &params.crust;
    let bt: BoundaryType = boundary_type(&sample.a, neighbor, sample.q, normal, params);
    let chi_a = continent_fraction(kappa_plate(&sample.a, sample.q, seed, params));
    let chi_n = continent_fraction(kappa_plate(neighbor, sample.q, seed, params));
    let k = boundary_weights(bt, chi_a, chi_n);
    let sigma_l = if upper_is_a(&sample.a, neighbor, seed) {
        d_l
    } else {
        -d_l
    };

    let detail_seed = subseed(seed, SALT_CRUST_DETAIL);
    let island_noise = 0.5
        + 0.5
            * fbm2(
                detail_seed,
                sample.q.x,
                sample.q.z,
                c.island_noise_lambda_l,
                2,
                0.5,
            );
    let transform_noise = fbm2(
        splitmix64(detail_seed),
        sample.q.x,
        sample.q.z,
        c.u7_noise_lambda_l,
        2,
        0.5,
    );

    let u1 = c.h_col_km * gauss(d_l, c.w_col_l)
        + c.plateau_factor * c.h_col_km * plateau_shape(sigma_l, c.w_col_l);
    let u2 = -c.h_trench_km * gauss(sigma_l + c.d_trench_l, c.w_trench_l)
        + c.h_arc_km * gauss(sigma_l - c.d_arc_l, c.w_arc_l);
    let u3 = -c.h_trench_km * gauss(sigma_l + c.d_trench_l, c.w_trench_l)
        + c.h_island_km
            * gauss(sigma_l - c.d_island_l, c.w_island_l)
            * (c.island_noise_base + c.island_noise_amp * island_noise);
    let u4 = c.h_ridge_km * gauss(d_l, c.w_ridge_l) - c.h_axial_km * gauss(d_l, c.w_axial_l);
    let u5 = -c.h_rift_km * (1.0 + bt.div) * gauss(d_l, c.w_rift_l)
        + c.h_shoulder_km * gauss(d_l - c.rift_shoulder_offset * c.w_rift_l, c.w_rift_l);
    let u6 = 0.5 * c.h_ridge_km * gauss(sigma_l + c.u6_offset_l, c.w_ridge_l)
        - c.u6_drop_km * gauss(sigma_l, c.u6_width_l);
    let u7 = c.u7_amplitude_km * gauss(d_l, c.u7_width_l) * transform_noise;

    let profiles = [u1, u2, u3, u4, u5, u6, u7];
    let mut u = 0.0;
    for (ki, ui) in k.iter().zip(profiles.iter()) {
        u += ki * ui;
    }

    let positive = k[0] * c.h_col_km * gauss(d_l, c.w_col_l)
        + k[1] * c.h_arc_km * gauss(sigma_l - c.d_arc_l, c.w_arc_l)
        + k[2]
            * c.h_island_km
            * gauss(sigma_l - c.d_island_l, c.w_island_l)
            * (c.island_noise_base + c.island_noise_amp * island_noise)
        + k[3] * c.h_ridge_km * gauss(d_l, c.w_ridge_l)
        + k[4] * c.h_shoulder_km * gauss(d_l - c.rift_shoulder_offset * c.w_rift_l, c.w_rift_l)
        + k[5] * 0.5 * c.h_ridge_km * gauss(sigma_l + c.u6_offset_l, c.w_ridge_l);

    BoundaryContrib {
        u: weight * u,
        positive: weight * positive,
        plateau: weight * k[0] * plateau_shape(sigma_l, c.w_col_l),
        k7_gauss: weight * k[6] * gauss(d_l, c.mountain_edge_l),
        sigma_l,
        upper: upper_is_a(&sample.a, neighbor, seed),
    }
}

/// Горячие точки и цепочки затонувших гор (7.4.4). Смотрим ячейки 3×3 вокруг
/// точки, но для каждой близкой точки, которая действительно близка, один раз
/// запрашиваем плиту — так стоимость остаётся локальной и небольшой.
pub fn hotspot_height(point: Vec2, seed: u64, params: &Params) -> f64 {
    let c = &params.crust;
    let grid = c.hotspot_cell_l;
    let ci = (point.x / grid).floor() as i64;
    let cj = (point.z / grid).floor() as i64;
    let salt = subseed(seed, SALT_HOTSPOT);
    let chain_reach = c.hotspot_chain_step_l * (c.hotspot_chain_count as f64 - 1.0);
    let consider = c.hotspot_consider_radius_l + chain_reach;
    let mut result = 0.0;
    for dj in -1..=1_i64 {
        for di in -1..=1_i64 {
            let cell = cell_hash(salt, ci + di, cj + dj, 0);
            if unit(cell) >= c.hotspot_probability {
                continue;
            }
            let mut rng = CellRng::from_hash(cell);
            let _ = rng.next_unit(); // то же число, что решило «есть точка»
            let h = Vec2::new(
                ((ci + di) as f64 + rng.next_range(c.hotspot_inset, 1.0 - c.hotspot_inset)) * grid,
                ((cj + dj) as f64 + rng.next_range(c.hotspot_inset, 1.0 - c.hotspot_inset)) * grid,
            );
            if h.dist2(point).sqrt() > consider {
                continue;
            }
            let (chi_h, velocity) = match plate_query(h, seed, params) {
                Some(sample) => (
                    continent_fraction(kappa_plate(&sample.a, sample.q, seed, params)),
                    sample.a.velocity_at(sample.q).normalized(),
                ),
                None => (0.0, Vec2::new(1.0, 0.0)),
            };
            for k in 0..c.hotspot_chain_count {
                let kf = k as f64;
                let center = h.sub(velocity.scale(kf * c.hotspot_chain_step_l));
                let radius = c.r_hotspot_l * (1.0 + c.hotspot_radius_growth * kf);
                let amplitude = c.h_hotspot_km
                    * (1.0 - c.hotspot_chain_decay * kf)
                    * (1.0 - c.hotspot_continental_factor * chi_h);
                result += amplitude * dexp(-point.dist2(center) / (radius * radius));
            }
        }
    }
    result
}

/// Полный расчёт слоя L2 в точке. `point` — макро-координата до искажения,
/// `sample` — результат `plate_query` в той же точке.
pub fn crust_from_sample(
    point: Vec2,
    sample: &PlateSample,
    seed: u64,
    params: &Params,
) -> CrustSample {
    let c = &params.crust;
    let kappa_l = kappa_global(sample.q, seed, params);
    let chi = continent_fraction(kappa_l);
    let mut bc = BoundaryContrib::default();
    let mut weights = [0.0; 7];
    let chi_a_plate = continent_fraction(kappa_plate(&sample.a, sample.q, seed, params));
    let chi_b_plate = sample
        .b
        .map(|b| continent_fraction(kappa_plate(&b, sample.q, seed, params)))
        .unwrap_or(0.0);
    let chi_c_plate = sample
        .c
        .map(|c| continent_fraction(kappa_plate(&c, sample.q, seed, params)))
        .unwrap_or(0.0);
    if let Some(b) = sample.b {
        let bt = boundary_type(&sample.a, &b, sample.q, sample.n_ab, params);
        let chi_b = continent_fraction(kappa_plate(&b, sample.q, seed, params));
        weights = boundary_weights(bt, chi, chi_b);
        // Первый вклад кладём напрямую, а не через default(): иначе σ в
        // диагностике всегда был бы нулём (Д-022).
        bc = boundary_contrib(
            sample,
            &b,
            sample.d_ab_l,
            sample.n_ab,
            sample.w_b,
            seed,
            params,
        );
    }
    if let Some(c_plate) = sample.c {
        bc = bc.add(boundary_contrib(
            sample,
            &c_plate,
            sample.d_ac_l,
            sample.n_ac,
            sample.w_c,
            seed,
            params,
        ));
    }

    let hotspots = hotspot_height(point, seed, params);
    let mountain = clamp01(
        (bc.positive + hotspots) / c.mountain_scale_km + c.mountain_transform_factor * bc.k7_gauss,
    );

    let detail_seed = subseed(seed, SALT_CRUST_DETAIL);
    let land = fbm2(
        detail_seed,
        point.x,
        point.z,
        c.detail_land_lambda_l,
        c.detail_octaves,
        c.detail_gain,
    );
    let ocean = fbm2(
        splitmix64(detail_seed),
        point.x,
        point.z,
        c.detail_ocean_lambda_l,
        c.detail_octaves,
        c.detail_gain,
    );
    let ridges = ridged2(
        splitmix64(splitmix64(detail_seed)),
        point.x,
        point.z,
        c.detail_mountain_lambda_l,
        c.detail_octaves,
        c.detail_gain,
    );
    let detail = chi * c.detail_land_km * land
        + (1.0 - chi) * c.detail_ocean_km * ocean
        + mountain * c.detail_mountain_km * ridges;

    let e_base_km = e_base(kappa_l, params);
    let e_km = e_base_km + bc.u + hotspots + detail;
    let iota_l = if e_km > 0.0 {
        kappa_l.max((c.iota_height_factor * e_km).min(c.iota_max_l))
    } else {
        0.0
    };

    CrustSample {
        kappa_l,
        e_km,
        mountain,
        plateau: clamp01(bc.plateau),
        iota_l,
        boundary_weights: weights,
        chi,
        e_base_km,
        boundary_u_km: bc.u,
        hotspot_km: hotspots,
        detail_km: detail,
        upper_is_a: bc.upper,
        sigma_b_l: bc.sigma_l,
        plate_ids: [
            sample.a.id,
            sample.b.map(|b| b.id).unwrap_or(0),
            sample.c.map(|c| c.id).unwrap_or(0),
        ],
        chi_plates: [chi_a_plate, chi_b_plate, chi_c_plate],
    }
}

#[cfg(test)]
mod tests {
    use super::{e_base, gauss, kappa_global, plateau_shape};
    use crate::Generator;
    use crate::hash::{splitmix64, unit};
    use crate::math::Vec2;
    use crate::mode::Mode;
    use crate::version::GenVersion;

    fn params() -> crate::Params {
        crate::Params::defaults(GenVersion::V0, Mode::Gameplay)
    }

    #[test]
    fn e_base_is_continuous_at_table_knots() {
        let p = params();
        assert!((e_base(0.0, &p)).abs() < 1e-12);
        assert!((e_base(-0.5, &p) + 0.2).abs() < 1e-12);
        assert!((e_base(-1.5, &p) + 4.5).abs() < 1e-12);
        assert!((e_base(-20.0, &p) + 4.5).abs() < 1e-12);
        for k in 0..200 {
            let x = k as f64 * 0.05 - 5.0;
            let a = e_base(x - 1e-7, &p);
            let b = e_base(x + 1e-7, &p);
            assert!((a - b).abs() < 1e-5, "разрыв в κ = {x}: {a} и {b}");
        }
        assert!(e_base(4.0, &p) > 0.0);
    }

    #[test]
    fn gauss_and_plateau_are_bounded() {
        assert_eq!(gauss(0.0, 1.0), 1.0);
        assert!(gauss(5.0, 1.0) < 1e-9);
        assert!(plateau_shape(0.0, 0.6) > 0.0);
        assert!(plateau_shape(-10.0, 0.6) >= 0.0);
    }

    #[test]
    fn crust_samples_are_finite_and_bounded() {
        let generator = Generator::new(42, Mode::Gameplay, GenVersion::V0).unwrap();
        let mut land = 0;
        let mut abyss = 0;
        for k in 0..400 {
            // Область ±160 L: при доле плит с материком 0,15 маленькое
            // окошко ±16 L может случайно не содержать ни одного материка.
            let x = unit(splitmix64(k)) * 80_000.0 - 40_000.0;
            let z = unit(splitmix64(k + 999_999)) * 80_000.0 - 40_000.0;
            let s = generator.crust_sample(x, z).unwrap();
            assert!(s.e_km.is_finite(), "e = {}", s.e_km);
            assert!(s.kappa_l.is_finite());
            assert!((0.0..=1.0).contains(&s.mountain));
            assert!((0.0..=1.0).contains(&s.plateau));
            assert!(s.iota_l >= 0.0);
            assert!(s.e_km > -11.0 && s.e_km < 9.0, "e = {}", s.e_km);
            if s.e_km > 0.0 {
                land += 1;
            }
            if s.e_km < -3.0 {
                abyss += 1;
            }
        }
        // Гипсометрия должна быть двумодальной: есть и суша, и абиссаль.
        assert!(land > 20, "суши слишком мало: {land}");
        assert!(abyss > 100, "абиссали слишком мало: {abyss}");
    }

    /// Д-012: смена окна ячеек не должна рисовать прямую линию на карте.
    #[test]
    fn kappa_global_has_no_cell_window_seams() {
        let p = params();
        let g = p.plates.cell_g_l;
        for k in 0..40 {
            let z = k as f64 * 1.7 - 30.0;
            let a = kappa_global(Vec2::new(g - 1e-6, z), 42, &p);
            let b = kappa_global(Vec2::new(g + 1e-6, z), 42, &p);
            // Поле гладкое, поэтому между двумя близкими точками допустим
            // небольшой наклон; разрыв окна дал бы скачок порядка 0,1–2 L.
            assert!(
                (a - b).abs() < 1e-3,
                "разрыв на границе ячеек: {a} и {b} при z = {z}"
            );
        }
    }
}
