//! T144: собственная функция высоты (полная I2, PLAN-REVISION §6.7).
//!
//! Это не ванильные сплайны Mojang и не их копия: своя таблица высот по
//! полосам E и PV, свой ocean/coast переход, factor/jaggedness по типу
//! рельефа. В научном режиме поверх таблицы идёт реальная макровысота `e_km`.
//! Вырез русел/котловин под зеркало воды добавляет T145; здесь считается
//! крупная форма поверхности, которую предпросмотр может показать точно.

use crate::hydrology::HydrologySample;
use crate::math::{clamp, clamp01, lerp, piecewise, smoothstep};
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
    /// T145: после with_water это дно русла/котловины, вырезанное под зеркало.
    pub surface_y: f64,
    /// T145: зеркало воды в блоках; вне рек/озёр — уровень моря.
    pub water_y: f64,
    /// T145: дно под зеркалом (mirror - depth), до смешивания с берегом.
    pub bed_y: f64,
    /// T145: сила выреза русла/озера, 0…1.
    pub water_strength: f64,
}

impl Default for HeightSample {
    fn default() -> Self {
        Self {
            offset: (SEA_LEVEL - SURFACE_BASE) / OFFSET_SCALE,
            factor: 0.0,
            jaggedness: 0.0,
            surface_y: SEA_LEVEL,
            water_y: SEA_LEVEL,
            bed_y: SEA_LEVEL,
            water_strength: 0.0,
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
        water_y: SEA_LEVEL,
        bed_y: y,
        water_strength: 0.0,
    }
}

/// T145: вырез русел/котловин под зеркало воды (§5.9). Река берёт монотонное
/// зеркало T145 из ChannelHit и не поднимается выше локального берега −1.
/// Озеро стоит плоским зеркалом на уровне перелива; дно = зеркало − глубина.
/// Это своя высота для предпросмотра/будущего R2; игра ставит воду через T143.
pub fn with_water(
    mut height: HeightSample,
    hydrology: &HydrologySample,
    params: &Params,
) -> HeightSample {
    let h = &params.height;
    let land_y = height.surface_y;
    let mut best = 0.0_f64;
    let mut water_y = SEA_LEVEL;
    let mut bed_y = land_y;
    if let Some(hit) = hydrology.channels.nearest {
        let cut = clamp01(hydrology.channels.core.max(hit.core_strength));
        if cut > 0.0 {
            let mirror = hit.mirror_y.round().min(land_y - 1.0);
            if mirror >= h.min_y {
                let depth = hit.depth_blocks.max(1.0);
                let bed = (mirror - depth).max(h.min_y);
                if cut > best {
                    best = cut;
                    water_y = mirror;
                    bed_y = bed;
                }
            }
        }
    }
    if let Some(hit) = hydrology.lakes.water_hit.filter(|hit| hit.inside) {
        if let Some(level) = hit.level_km {
            let cut = clamp01(hit.water_strength);
            if cut > 0.0 {
                let mirror = (SEA_LEVEL + level.max(0.0) * h.water_km_to_blocks)
                    .round()
                    .min(h.max_y - 1.0);
                let depth = clamp(
                    hit.max_depth_km * h.water_km_to_blocks,
                    1.0,
                    h.lake_depth_max_blocks,
                );
                let bed = (mirror - depth).max(h.min_y);
                if cut > best || (cut == best && bed < bed_y) {
                    best = cut;
                    water_y = mirror;
                    bed_y = bed;
                }
            }
        }
    }
    if best > 0.0 {
        height.surface_y = lerp(land_y, bed_y, best);
        height.water_y = water_y;
        height.bed_y = bed_y;
        height.water_strength = best;
        height.offset = (height.surface_y - SURFACE_BASE) / OFFSET_SCALE;
    }
    height
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
        || !h.water_km_to_blocks.is_finite()
        || h.water_km_to_blocks <= 0.0
        || !h.lake_depth_max_blocks.is_finite()
        || h.lake_depth_max_blocks < 1.0
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

    fn base_at(y: f64) -> HeightSample {
        HeightSample {
            surface_y: y,
            offset: (y - SURFACE_BASE) / OFFSET_SCALE,
            bed_y: y,
            ..HeightSample::default()
        }
    }

    fn channel_hit(mirror_y: f64, depth_blocks: f64) -> crate::channel::ChannelHit {
        crate::channel::ChannelHit {
            plate_id: 1,
            segment: 0,
            reach: 0,
            distance_l: 0.0,
            discharge_mm_l2: 100.0,
            core_l: 0.02,
            valley_l: 0.1,
            depth_blocks,
            core_strength: 1.0,
            valley_strength: 1.0,
            delta_strength: 0.0,
            end: crate::channel::ReachEnd::Confluence,
            mouth_distance_l: 1.0,
            mirror_y,
        }
    }

    #[test]
    fn river_cut_keeps_water_below_the_local_bank() {
        let p = params(Mode::Gameplay);
        let mut hydro = HydrologySample::default();
        hydro.channels.core = 1.0;
        hydro.channels.nearest = Some(channel_hit(80.0, 2.0));
        let cut = with_water(base_at(100.0), &hydro, &p);
        assert_eq!(cut.water_y, 80.0);
        assert_eq!(cut.bed_y, 78.0);
        assert_eq!(cut.surface_y, 78.0);
        assert_eq!(cut.water_strength, 1.0);
        // Берег ниже зеркала: вода не должна висеть над берегом.
        let low = with_water(base_at(70.0), &hydro, &p);
        assert_eq!(low.water_y, 69.0);
        assert!(low.surface_y <= low.water_y - 1.0);
    }

    #[test]
    fn lake_mirror_is_flat_at_spill_and_bed_is_below_it() {
        let p = params(Mode::Gameplay);
        let mut hydro = HydrologySample::default();
        hydro.lakes.water_hit = Some(crate::lake::LakeHit {
            plate_id: 1,
            lake: 0,
            depression: 0,
            kind: crate::lake::LakeKind::Lake,
            distance_l: 0.0,
            inside: true,
            level_km: Some(0.1),
            max_depth_km: 0.05,
            frozen: false,
            rift: false,
            high: true,
            water_strength: 1.0,
            shore_strength: 1.0,
        });
        let cut = with_water(base_at(80.0), &hydro, &p);
        assert_eq!(cut.water_y, 69.0); // 63 + 0.1*64 = 69.4 -> 69
        assert!((cut.bed_y - 65.8).abs() < 1e-9);
        assert_eq!(cut.surface_y, cut.bed_y);
        assert_eq!(cut.water_strength, 1.0);
    }
}
