//! T144: собственная функция высоты (полная I2, PLAN-REVISION §6.7).
//!
//! Это не ванильные сплайны Mojang и не их копия: своя таблица высот по
//! полосам E и PV, свой ocean/coast переход, factor/jaggedness по типу
//! рельефа. В научном режиме поверх таблицы идёт реальная макровысота `e_km`.
//! Вырез русел/котловин под зеркало воды добавляет T145; здесь считается
//! крупная форма поверхности, которую предпросмотр может показать точно.

use crate::math::{clamp, piecewise, smoothstep};
use crate::params::Params;

/// Уровень моря в блоках. Вертикаль Minecraft: 1 блок ≈ 1 м.
pub const SEA_LEVEL: f64 = 63.0;
/// База ванильной формулы offset; сохранена для совместимости предпросмотра.
pub const SURFACE_BASE: f64 = 63.52;
pub const OFFSET_SCALE: f64 = 128.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeightSample {
    /// Плотность `overworld/offset`: (surface_y - SURFACE_BASE) / OFFSET_SCALE.
    pub offset: f64,
    pub factor: f64,
    pub jaggedness: f64,
    /// Крупная форма поверхности в блоках, до ванильной 3D-фактуры/пещер.
    pub surface_y: f64,
}

impl Default for HeightSample {
    fn default() -> Self {
        Self {
            offset: (SEA_LEVEL - SURFACE_BASE) / OFFSET_SCALE,
            factor: 0.0,
            jaggedness: 0.0,
            surface_y: SEA_LEVEL,
        }
    }
}

/// Непрерывное значение по полосам E. Узлы — границы полос_stats; на границе
/// берём среднее соседних полос, чтобы таблица не давала обрыв на стыке E.
fn by_erosion(e: f64, rows: &[f64; 7], knots: &[f64; 8]) -> f64 {
    let mut values = [0.0; 8];
    values[0] = rows[0];
    values[7] = rows[6];
    for i in 1..7 {
        values[i] = 0.5 * (rows[i - 1] + rows[i]);
    }
    piecewise(knots, &values, e)
}

/// T144: высота/factor/jaggedness из тех же C/E/PV, что выбирают биом.
/// `cold`/`warm` — веса холодной/тёплой горной морфозоны T121; они делают
/// холодные пики острее, не переключая тип рельефа скачком.
pub fn sample(
    c: f64,
    e: f64,
    pv: f64,
    e_km: f64,
    cold: f64,
    warm: f64,
    params: &Params,
) -> HeightSample {
    debug_assert!(validate(params).is_ok());
    let h = &params.height;
    let pv = clamp(pv, h.pv_knots[0], h.pv_knots[h.pv_knots.len() - 1]);
    let mut rows = [0.0; 7];
    for (k, row) in h.land_y.iter().enumerate() {
        rows[k] = piecewise(&h.pv_knots, row, pv);
    }
    let land_y = by_erosion(e, &rows, &h.e_knots);
    let ocean_y = piecewise(&h.ocean_c, &h.ocean_y, c);
    let threshold = params.climate.ocean_c_threshold;
    let coast = smoothstep(threshold - h.coast_width_c, threshold + h.coast_width_c, c);
    let mut y = ocean_y + (land_y - ocean_y) * coast;
    // Научный режим: реальная макровысота поверх своей таблицы. Gameplay
    // остаётся на блочной таблице; e_gain по умолчанию 0.
    y += h.e_gain_blocks_per_km * e_km;
    let y = clamp(y, h.min_y, h.max_y);
    let factor = clamp(by_erosion(e, &h.factor, &h.e_knots), 0.0, 1.0);
    let jaggedness = clamp(
        by_erosion(e, &h.jaggedness, &h.e_knots) + h.cold_jagged_gain * cold
            - h.warm_jagged_drop * warm,
        0.0,
        1.0,
    );
    HeightSample {
        offset: (y - SURFACE_BASE) / OFFSET_SCALE,
        factor,
        jaggedness,
        surface_y: y,
    }
}

pub fn validate(params: &Params) -> Result<(), &'static str> {
    let h = &params.height;
    let increasing = |xs: &[f64]| xs.windows(2).all(|w| w[0].is_finite() && w[1] > w[0]);
    if !increasing(&h.e_knots) || !increasing(&h.pv_knots) || !increasing(&h.ocean_c) {
        return Err("height: узлы таблиц должны строго возрастать");
    }
    if h.land_y.iter().flatten().any(|v| !v.is_finite())
        || h.ocean_y.iter().any(|v| !v.is_finite())
        || h.factor
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        || h.jaggedness
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err("height: таблицы высот/factor/jaggedness должны быть конечными");
    }
    if !h.coast_width_c.is_finite()
        || h.coast_width_c <= 0.0
        || !h.e_gain_blocks_per_km.is_finite()
        || !h.cold_jagged_gain.is_finite()
        || !(0.0..=1.0).contains(&h.cold_jagged_gain)
        || !h.warm_jagged_drop.is_finite()
        || !(0.0..=1.0).contains(&h.warm_jagged_drop)
        || !h.min_y.is_finite()
        || !h.max_y.is_finite()
        || h.min_y >= h.max_y
    {
        return Err("height: переход берега/вертикальный масштаб/пределы некорректны");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::Mode;
    use crate::version::GenVersion;

    fn params(mode: Mode) -> Params {
        Params::defaults(GenVersion::V1Dev, mode)
    }

    #[test]
    fn ocean_is_below_sea_and_land_is_above() {
        let p = params(Mode::Gameplay);
        let sea = sample(-0.6, 0.3, 0.0, -2.0, 0.0, 0.0, &p);
        let land = sample(0.4, 0.3, 0.0, 0.5, 0.0, 0.0, &p);
        assert!(sea.surface_y < SEA_LEVEL, "океан {:?}", sea);
        assert!(land.surface_y > SEA_LEVEL, "суша {:?}", land);
    }

    #[test]
    fn negative_pv_cuts_a_plateau_valley() {
        let p = params(Mode::Gameplay);
        let valley = sample(0.4, -0.3, -1.0, 0.5, 0.0, 0.0, &p);
        let peak = sample(0.4, -0.3, 1.0, 0.5, 0.0, 0.0, &p);
        assert!(valley.surface_y < peak.surface_y - 20.0);
        assert!(valley.surface_y < 60.0, "каньон плато {:?}", valley);
    }

    #[test]
    fn scientific_mode_adds_real_macro_elevation() {
        let p = params(Mode::Scientific);
        let low = sample(0.4, -0.5, 0.3, 0.0, 0.0, 0.0, &p);
        let high = sample(0.4, -0.5, 0.3, 2.0, 0.0, 0.0, &p);
        assert!(high.surface_y > low.surface_y + 20.0);
        let game = params(Mode::Gameplay);
        let a = sample(0.4, -0.5, 0.3, 0.0, 0.0, 0.0, &game);
        let b = sample(0.4, -0.5, 0.3, 2.0, 0.0, 0.0, &game);
        assert_eq!(a.surface_y.to_bits(), b.surface_y.to_bits());
    }

    #[test]
    fn cold_mountains_are_jaggeder_than_warm_and_fields_are_bounded() {
        let p = params(Mode::Gameplay);
        let cold = sample(0.4, -0.9, 0.7, 1.0, 1.0, 0.0, &p);
        let warm = sample(0.4, -0.9, 0.7, 1.0, 0.0, 1.0, &p);
        assert!(cold.jaggedness > warm.jaggedness);
        for k in 0..200 {
            let t = k as f64 / 199.0;
            let s = sample(
                -1.0 + 2.0 * t,
                -1.0 + 2.0 * t,
                -1.0 + 2.0 * t,
                -4.0 + 8.0 * t,
                t,
                1.0 - t,
                &p,
            );
            assert!(s.surface_y.is_finite());
            assert!((p.height.min_y..=p.height.max_y).contains(&s.surface_y));
            assert!((0.0..=1.0).contains(&s.factor));
            assert!((0.0..=1.0).contains(&s.jaggedness));
            assert_eq!(
                s.offset.to_bits(),
                ((s.surface_y - SURFACE_BASE) / OFFSET_SCALE).to_bits()
            );
        }
    }

    #[test]
    fn height_keys_are_v1_dev_only_and_validate() {
        let old = Params::defaults(GenVersion::V0, Mode::Gameplay);
        assert!(!old.canonical_text().contains("height."));
        let dev = params(Mode::Gameplay);
        assert!(dev.canonical_text().contains("height.land_y="));
        validate(&dev).unwrap();
        let mut bad = dev.clone();
        bad.height.coast_width_c = 0.0;
        assert!(validate(&bad).is_err());
    }
}
