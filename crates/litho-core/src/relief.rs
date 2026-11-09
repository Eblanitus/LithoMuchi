//! Потенциал рельефа A (PLAN-REVISION 6.2, T120).
//!
//! Слой собирает геологические формы в безразмерный потенциал `A ∈ [0, 1]`.
//! Источники объединяются максимумом и плавными смесями, без дискретных
//! переключателей (урок Д-022). Котловины дают отрицательный вклад, грабены
//! рифтов выравнивают дно и поднимают борта. Это ещё не игровая высота:
//! перевод в E/PV/C делает T122, подбор гипсометрии — T123.

use crate::crust::CrustSample;
use crate::drainage::RiverNetError;
use crate::math::{clamp01, dexp, lerp, smoothstep};
use crate::noise::{fbm2_unit, ridged2};
use crate::params::Params;
use crate::plates::PlateSample;
use crate::seed::{SALT_RELIEF_BASIN, SALT_RELIEF_HILLS, SALT_RELIEF_PALEO, subseed};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReliefSample {
    /// Итоговый потенциал рельефа после котловин и выравнивания рифта.
    pub a: f64,
    pub foothill: f64,
    pub paleosuture: f64,
    pub craton: f64,
    pub escarpment: f64,
    pub volcanic: f64,
    pub hills: f64,
    /// Сила отрицательного вклада котловины, 0…1.
    pub basin: f64,
    /// Интерполированная глубина впадины T110, км.
    pub basin_depth_km: f64,
    /// Сила ровного дна грабена и его бортов.
    pub rift_floor: f64,
    pub rift_shoulder: f64,
}

pub fn validate(params: &Params) -> Result<(), RiverNetError> {
    if !params.version.is_water_relief_dev() {
        return Err(RiverNetError::UnsupportedVersion(params.version));
    }
    params
        .relief
        .validate()
        .map_err(RiverNetError::InvalidParameters)?;
    let c = &params.crust;
    if !c.w_col_l.is_finite() || c.w_col_l <= 0.0 {
        return Err(RiverNetError::InvalidParameters(
            "crust.w_col_l должен быть положительным",
        ));
    }
    if !c.w_rift_l.is_finite() || c.w_rift_l <= 0.0 {
        return Err(RiverNetError::InvalidParameters(
            "crust.w_rift_l должен быть положительным",
        ));
    }
    if !c.h_hotspot_km.is_finite() || c.h_hotspot_km <= 0.0 {
        return Err(RiverNetError::InvalidParameters(
            "crust.h_hotspot_km должен быть положительным",
        ));
    }
    Ok(())
}

fn gauss(x: f64, width: f64) -> f64 {
    let t = x / width;
    dexp(-(t * t))
}

/// Чистая функция точки. `basin_depth_km` приходит из уже построенной сети
/// T110 (`RiverNet::depression_depth_km`); сам слой сеть не строит и не
/// создаёт цикл зависимости «сток → рельеф → сток».
pub fn sample(
    plate: &PlateSample,
    crust: &CrustSample,
    basin_depth_km: f64,
    seed: u64,
    params: &Params,
) -> ReliefSample {
    debug_assert!(validate(params).is_ok());
    let r = &params.relief;
    let q = plate.q;
    let land = smoothstep(0.0, r.land_e_fade_km, crust.e_km);
    let continental = land * smoothstep(r.continental_chi_lo, r.continental_chi_hi, crust.chi);
    let inactive = smoothstep(r.active_boundary_lo, r.active_boundary_hi, plate.d_ab_l);

    // Предгорья: внешний склон орогена, полоса 0,05 < M < 0,5.
    let m = crust.mountain;
    let rise = smoothstep(r.foothill_m_lo, r.foothill_m_lo + r.foothill_fade, m);
    let fall = 1.0 - smoothstep(r.foothill_m_hi - r.foothill_fade, r.foothill_m_hi, m);
    let amplitude = lerp(
        r.foothill_a_min,
        r.foothill_a_max,
        smoothstep(r.foothill_m_lo, r.foothill_m_hi, m),
    );
    let foothill = amplitude * rise * fall * land;

    // Палеошвы: узкие полосы гребневого шума на старой материковой коре.
    let ridge = ridged2(
        subseed(seed, SALT_RELIEF_PALEO),
        q.x,
        q.z,
        r.paleo_lambda_l,
        3,
        0.5,
    );
    let band = 1.0 - smoothstep(r.paleo_band_lo, r.paleo_band_hi, 1.0 - ridge);
    let paleosuture = lerp(r.paleo_a_min, r.paleo_a_max, ridge) * band * continental * inactive;

    // Кратоны: глубь материка, низкий потенциал вместо молодых гор.
    let depth = smoothstep(r.craton_iota_lo, r.craton_iota_hi, crust.iota_l);
    let craton = lerp(r.craton_a_min, r.craton_a_max, depth) * continental * inactive;

    // Уступы плато: большой |∇Pl| из знаковых профилей коры.
    let escarpment = r.escarpment_a
        * smoothstep(
            r.escarpment_gradient_lo,
            r.escarpment_gradient_hi,
            crust.plateau_gradient,
        )
        * land;

    // Вулканические поля: горячие точки уже дают конусы и цепочки в коре.
    let hot = clamp01(crust.hotspot_km / params.crust.h_hotspot_km);
    let volcanic = lerp(r.volcanic_a_min, r.volcanic_a_max, hot)
        * smoothstep(r.volcanic_hotspot_lo, r.volcanic_hotspot_hi, hot);

    // Холмистые районы: единичный fBm, положительная часть как потенциал.
    let hills_noise = fbm2_unit(
        subseed(seed, SALT_RELIEF_HILLS),
        q.x,
        q.z,
        r.hills_lambda_l,
        3,
        0.5,
    );
    let hills = r.hills_a * smoothstep(r.hills_noise_lo, r.hills_noise_hi, hills_noise) * land;

    // Котловины: фактическая впадина стока важнее шумового сигнала.
    let basin_noise = fbm2_unit(
        subseed(seed, SALT_RELIEF_BASIN),
        q.x,
        q.z,
        r.basin_lambda_l,
        3,
        0.5,
    );
    let basin_depth_km = if basin_depth_km.is_finite() {
        basin_depth_km.max(0.0)
    } else {
        0.0
    };
    let basin_signal = smoothstep(r.basin_noise_lo, r.basin_noise_hi, basin_noise) * continental;
    let basin_depth = smoothstep(0.0, r.basin_depth_km, basin_depth_km) * land;
    let basin = basin_signal.max(basin_depth);

    // Грабены: ровное дно и крутые борта по весу рифта k5.
    let rift = crust.rift_weight;
    let w_rift = params.crust.w_rift_l;
    let d = plate.d_ab_l;
    let rift_floor = rift * (1.0 - smoothstep(0.0, r.rift_floor_factor * w_rift, d));
    let rift_shoulder =
        rift * gauss(
            d - r.rift_shoulder_offset_factor * w_rift,
            r.rift_shoulder_width_factor * w_rift,
        ) * r.rift_shoulder_a
            * land;

    // Максимум источников непрерывен; кратон и холмы вмешаны плавно, чтобы
    // слабая форма не стирала щит и не давала ступень на границе источника.
    let structural = foothill
        .max(paleosuture)
        .max(escarpment)
        .max(volcanic)
        .max(rift_shoulder);
    let mut a = lerp(craton, structural, structural);
    a += hills * (1.0 - a);
    a = lerp(
        a,
        r.rift_floor_a,
        clamp01(rift_floor * r.rift_flat_strength),
    );
    a = clamp01(a - r.basin_strength * basin);

    ReliefSample {
        a,
        foothill,
        paleosuture,
        craton,
        escarpment,
        volcanic,
        hills,
        basin,
        basin_depth_km,
        rift_floor,
        rift_shoulder,
    }
}

#[cfg(test)]
mod tests {
    use super::{sample, validate};
    use crate::crust::crust_from_sample;
    use crate::math::Vec2;
    use crate::mode::Mode;
    use crate::params::Params;
    use crate::plates::plate_query;
    use crate::version::GenVersion;

    fn params() -> Params {
        Params::defaults(GenVersion::V1Dev, Mode::Gameplay)
    }

    #[test]
    fn defaults_validate_and_v0_is_rejected() {
        let p = params();
        assert!(validate(&p).is_ok());
        let v0 = Params::defaults(GenVersion::V0, Mode::Gameplay);
        assert!(validate(&v0).is_err());
        let mut bad = p.clone();
        bad.relief.paleo_lambda_l = 0.0;
        assert!(validate(&bad).is_err());
        let mut bad = p.clone();
        bad.relief.foothill_a_min = 0.9;
        bad.relief.foothill_a_max = 0.1;
        assert!(validate(&bad).is_err());
        let mut bad = p;
        bad.relief.basin_strength = 1.5;
        assert!(validate(&bad).is_err());
    }

    #[test]
    fn potential_is_bounded_on_a_transect() {
        let p = params();
        for k in 0..600 {
            let t = k as f64 * 0.05;
            let point = Vec2::new(t * 3.0, -t * 1.7);
            let plate = plate_query(point, 42, &p).unwrap();
            let crust = crust_from_sample(point, &plate, 42, &p);
            let depth = 0.2 * crate::math::dexp(-((t - 10.0) * (t - 10.0)));
            let r = sample(&plate, &crust, depth, 42, &p);
            assert!(r.a.is_finite() && (0.0..=1.0).contains(&r.a));
            for value in [
                r.foothill,
                r.paleosuture,
                r.craton,
                r.escarpment,
                r.volcanic,
                r.hills,
                r.basin,
                r.rift_floor,
                r.rift_shoulder,
            ] {
                assert!(value.is_finite() && (0.0..=1.0).contains(&value));
            }
        }
    }

    #[test]
    fn potential_has_no_step_between_close_points() {
        let p = params();
        let eps = 1e-6;
        for k in 0..200 {
            let t = k as f64 * 0.13;
            let point = Vec2::new(t * 2.7 - 20.0, -t * 1.9 + 7.0);
            let shifted = point.add(Vec2::new(eps, -eps));
            let a = sample(
                &plate_query(point, 42, &p).unwrap(),
                &crust_from_sample(point, &plate_query(point, 42, &p).unwrap(), 42, &p),
                0.0,
                42,
                &p,
            )
            .a;
            let b = sample(
                &plate_query(shifted, 42, &p).unwrap(),
                &crust_from_sample(shifted, &plate_query(shifted, 42, &p).unwrap(), 42, &p),
                0.0,
                42,
                &p,
            )
            .a;
            assert!((a - b).abs() < 1e-4, "ступень A в {point:?}: {}", a - b);
        }
    }

    #[test]
    fn basin_lowers_potential_and_rift_flattens_floor() {
        let p = params();
        let point = Vec2::new(12.3, -4.5);
        let plate = plate_query(point, 42, &p).unwrap();
        let mut crust = crust_from_sample(point, &plate, 42, &p);
        crust.e_km = 0.4;
        crust.chi = 1.0;
        crust.iota_l = 1.0;
        crust.mountain = 0.0;
        crust.plateau = 0.0;
        crust.plateau_gradient = 0.0;
        crust.hotspot_km = 0.0;
        crust.boundary_weights = [0.0; 7];
        crust.rift_weight = 0.0;
        let dry = sample(&plate, &crust, 0.0, 42, &p);
        let wet = sample(&plate, &crust, 0.5, 42, &p);
        assert!(wet.basin >= dry.basin);
        assert!(wet.a <= dry.a);

        crust.rift_weight = 1.0;
        let mut floor_plate = plate;
        floor_plate.d_ab_l = 0.0;
        let floor = sample(&floor_plate, &crust, 0.0, 42, &p);
        let mut shoulder_plate = plate;
        shoulder_plate.d_ab_l = p.relief.rift_shoulder_offset_factor * p.crust.w_rift_l;
        let shoulder = sample(&shoulder_plate, &crust, 0.0, 42, &p);
        assert!(floor.rift_floor > 0.9);
        assert!(floor.a < shoulder.a);
        assert!(shoulder.rift_shoulder > 0.5);
    }

    #[test]
    fn foothills_live_on_the_outer_orogen_slope() {
        let p = params();
        let point = Vec2::new(2.0, 3.0);
        let plate = plate_query(point, 7, &p).unwrap();
        let mut crust = crust_from_sample(point, &plate, 7, &p);
        crust.e_km = 0.5;
        crust.chi = 1.0;
        crust.hotspot_km = 0.0;
        crust.boundary_weights = [0.0; 7];
        crust.rift_weight = 0.0;
        crust.mountain = 0.0;
        assert!(sample(&plate, &crust, 0.0, 7, &p).foothill < 1e-9);
        crust.mountain = 0.25;
        let mid = sample(&plate, &crust, 0.0, 7, &p).foothill;
        assert!(mid > 0.4 && mid < 0.8);
        crust.mountain = 0.9;
        assert!(sample(&plate, &crust, 0.0, 7, &p).foothill < 1e-9);
    }
}
