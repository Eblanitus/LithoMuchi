//! Перевод сети стока и озёр в C/E/W и биом (PLAN-REVISION 5.6–5.7, T115).
//!
//! Это подход A / R1: ванильная высота остаётся ванильной, собственного PV и
//! воды выше моря здесь нет. Низкое озеро использует речной биом, большое —
//! океанский, болото — E6. Высокое озеро до R2 не притворяется водой на море:
//! сохраняется честно помеченный запасной путь «сухая котловина / болото».

use crate::channel::{ChannelQuery, ReachEnd};
use crate::classify::{ClassSample, VanillaCell, biome_cell, ocean_cell};
use crate::climate::ClimateSample;
use crate::drainage::RiverNetError;
use crate::lake::{LakeKind, LakeQuery};
use crate::math::{clamp, clamp01, lerp, smoothstep};
use crate::params::Params;
use crate::vanilla::{VanillaSample, continentalness};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SurfaceKind {
    #[default]
    Land,
    River,
    Lake,
    InlandSea,
    Swamp,
    DryBasin,
    /// Вода диагностики T114 есть, но её уровень требует R2. В полях только суша.
    DeferredWater,
}

impl SurfaceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Land => "land",
            Self::River => "river",
            Self::Lake => "lake",
            Self::InlandSea => "inland_sea",
            Self::Swamp => "swamp",
            Self::DryBasin => "dry_basin",
            Self::DeferredWater => "deferred_water",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HydrologySample {
    pub channels: ChannelQuery,
    pub lakes: LakeQuery,
    pub kind: SurfaceKind,
    /// Сила речного биома/среза W; океанское море и болото сюда не входят.
    pub river: f64,
    pub valley: f64,
    /// Геометрическая вода внутри контура, включая болото и внутреннее море.
    pub water: f64,
    pub floodplain: f64,
    pub swamp: f64,
    pub dry_basin: f64,
    pub estuary: f64,
    pub r2_deferred: f64,
    /// T116: сила галерейного леса после сухости и выхода из океана.
    pub gallery: f64,
    pub networks_considered: usize,
}

pub fn validate(params: &Params) -> Result<(), RiverNetError> {
    let h = &params.hydrology;
    let values = [h.bank_fraction, h.field_fade_l, h.estuary_elevation_km];
    if values.iter().any(|v| !v.is_finite() || *v <= 0.0)
        || ![
            h.floodplain_e,
            h.swamp_e,
            h.dry_basin_e,
            h.estuary_c,
            h.inland_sea_c,
        ]
        .iter()
        .all(|v| v.is_finite() && (-1.0..=1.0).contains(v))
        || !h.r1_max_level_km.is_finite()
        || h.bank_fraction > 1.0
        || h.r1_max_level_km < 0.0
        || h.estuary_c >= params.climate.ocean_c_threshold
        || h.inland_sea_c >= params.climate.ocean_c_threshold
        || !h.gallery_q_min_mm_l2.is_finite()
        || h.gallery_q_min_mm_l2 <= 0.0
        || !h.gallery_width_factor.is_finite()
        || h.gallery_width_factor <= 1.0
        || !h.gallery_dry_r_lo.is_finite()
        || !h.gallery_dry_r_hi.is_finite()
        || h.gallery_dry_r_lo < 0.0
        || h.gallery_dry_r_hi <= h.gallery_dry_r_lo
        || !h.gallery_h_gain.is_finite()
        || !(0.0..=2.0).contains(&h.gallery_h_gain)
    {
        return Err(RiverNetError::InvalidParameters(
            "гидрология: ширины/влажность положительны, E/C в [-1,1], моря ниже порога C",
        ));
    }
    Ok(())
}

/// Чистое разрешение геометрии T113/T114. Максимумы силы не смешиваются с
/// типом другой поверхности: water_hit и shore_hit выбраны детерминированно.
pub fn resolve(
    channels: ChannelQuery,
    lakes: LakeQuery,
    climate: &ClimateSample,
    params: &Params,
) -> HydrologySample {
    let h = &params.hydrology;
    let mut out = HydrologySample {
        channels,
        lakes,
        river: channels.core,
        valley: channels.valley,
        networks_considered: channels.networks_considered,
        ..HydrologySample::default()
    };
    // Классы озёр смешиваются максимумами сил: выбор одного shore_hit мог
    // скачком менять E, когда два берега почти равны (болото/котловина).
    out.water = lakes.r1_water;
    out.river = out.river.max(lakes.river_lake);
    out.valley = out.valley.max(lakes.shore);
    out.swamp = lakes.swamp;
    out.dry_basin = lakes.dry_basin;
    out.r2_deferred = lakes.deferred_water;
    let inside = lakes.water_hit.filter(|hit| hit.inside);
    out.kind = if lakes.inland_sea > 0.0
        || inside.is_some_and(|hit| {
            matches!(hit.kind, LakeKind::InlandSea | LakeKind::SeaConnected)
                && hit.level_km.is_some_and(|level| level <= h.r1_max_level_km)
        }) {
        SurfaceKind::InlandSea
    } else if lakes.river_lake > 0.0
        || inside.is_some_and(|hit| {
            hit.kind == LakeKind::Lake
                && hit.level_km.is_some_and(|level| level <= h.r1_max_level_km)
        })
    {
        SurfaceKind::Lake
    } else if out.swamp > 0.0 {
        SurfaceKind::Swamp
    } else if out.r2_deferred > 0.0 {
        SurfaceKind::DeferredWater
    } else if out.dry_basin > 0.0 {
        SurfaceKind::DryBasin
    } else {
        SurfaceKind::Land
    };
    // Пойма не перекрывает горный каньон: множитель (1-M) из §5.7.
    out.floodplain = out.valley * (1.0 - clamp01(climate.surface.mountain));
    // E6 только у геометрических болот и дельт (§5.4/5.7). Влажная пойма
    // остаётся E4: резкие тени влаги T131 не должны прыгать в E на блок.
    out.swamp = out.swamp.max(channels.delta);
    if out.swamp > 0.0 && out.kind == SurfaceKind::Land {
        out.kind = SurfaceKind::Swamp;
    }
    if let Some(hit) = channels.nearest {
        if matches!(hit.end, ReachEnd::Sea | ReachEnd::Lake) {
            let q = params.channels.delta_q_min_mm_l2;
            let mouth = 1.0
                - smoothstep(
                    0.0,
                    params.channels.delta_length_max_l,
                    hit.mouth_distance_l,
                );
            let low = 1.0
                - smoothstep(
                    0.5 * h.estuary_elevation_km,
                    h.estuary_elevation_km,
                    climate.surface.e_km,
                );
            out.estuary =
                smoothstep(q, 2.0 * q, hit.discharge_mm_l2) * mouth * low * hit.valley_strength;
        }
    }
    // Б7: только суша и сухая зона. Вес R плавный; порог биомов отдельно
    // переводит сильный коридор в лес, численные T/H меняются без ступени.
    let dry = smoothstep(h.gallery_dry_r_lo, h.gallery_dry_r_hi, climate.precip.ratio);
    let land = smoothstep(0.0, 0.05, climate.surface.e_km);
    out.gallery = clamp01(channels.gallery * dry * land);
    if out.kind == SurfaceKind::Land && out.river > 0.0 {
        out.kind = SurfaceKind::River;
    }
    out.river = clamp01(out.river);
    out.valley = clamp01(out.valley);
    out.water = clamp01(out.water);
    out.floodplain = clamp01(out.floodplain);
    out.swamp = clamp01(out.swamp);
    out.dry_basin = clamp01(out.dry_basin);
    out.estuary = clamp01(out.estuary);
    out
}

/// Биом следует эффективной воде R1, а не сырому флагу высокого озера.
/// Климатические T/H сохраняются; замёрзшее малое озеро получает frozen_river.
pub fn classify_surface(
    base: ClassSample,
    hydrology: &HydrologySample,
    climate: &ClimateSample,
    params: &Params,
) -> ClassSample {
    let mut out = base;
    let cell = match hydrology.kind {
        SurfaceKind::InlandSea => Some(ocean_cell(climate.temperature.sst_c, params)),
        SurfaceKind::Land
        | SurfaceKind::River
        | SurfaceKind::DryBasin
        | SurfaceKind::DeferredWater
            if hydrology.gallery >= 0.5
                && hydrology.gallery > hydrology.river
                && hydrology.swamp == 0.0
                && base.cell.t > 0 =>
        {
            // Сильный коридор получает лесную ячейку; слабая кромка только
            // плавно поднимает H (и опускает t4 к t3 ниже, в apply_fields).
            Some(match base.zone.belt {
                0..=1 => base.cell,
                2 => VanillaCell {
                    t: 1,
                    h: 3,
                    biome: "taiga",
                },
                _ => VanillaCell {
                    t: base.cell.t.saturating_sub(1).max(2),
                    h: 2,
                    biome: "forest",
                },
            })
        }
        SurfaceKind::Land
        | SurfaceKind::River
        | SurfaceKind::Lake
        | SurfaceKind::DryBasin
        | SurfaceKind::DeferredWater
            if hydrology.river > 0.0 && hydrology.swamp == 0.0 =>
        {
            Some(
                if base.cell.t == 0 || climate.temperature.t_min <= params.lakes.frozen_t_min_c {
                    VanillaCell {
                        t: 0,
                        h: 0,
                        biome: "frozen_river",
                    }
                } else {
                    VanillaCell {
                        t: base.cell.t,
                        h: 0,
                        biome: "river",
                    }
                },
            )
        }
        SurfaceKind::Swamp | SurfaceKind::DeferredWater if hydrology.swamp > 0.0 => {
            Some(if base.cell.t >= 3 {
                VanillaCell {
                    t: base.cell.t,
                    h: 4,
                    biome: "mangrove_swamp",
                }
            } else {
                VanillaCell {
                    t: base.cell.t.clamp(1, 2),
                    h: 4,
                    biome: "swamp",
                }
            })
        }
        SurfaceKind::DryBasin | SurfaceKind::DeferredWater if hydrology.dry_basin > 0.0 => {
            let belt = base.zone.belt;
            // Солончака в ванили нет: тёплая котловина — саванна, остальные —
            // пустыня; правило холодной зимы сохраняет сухие равнины.
            Some(biome_cell(
                belt.max(3),
                if belt >= 4 { 3 } else { 1 },
                base.ratio.max(2.0),
                climate.temperature.t_min,
                params,
            ))
        }
        _ => None,
    };
    if let Some(cell) = cell {
        out.cell = cell;
        out.is_ocean = hydrology.kind == SurfaceKind::InlandSea;
        if out.is_ocean {
            out.zone = crate::classify::LifeZone {
                belt: cell.t,
                province: 1,
            };
        }
    }
    out
}

/// `base.w` ещё не подавлен рекой. Один общий множитель объединяет русло
/// сети, низкое озеро и локальную рифт/трансформную реку, без двойного среза.
pub fn apply_fields(
    mut base: VanillaSample,
    local_river: f64,
    hydrology: &HydrologySample,
    params: &Params,
) -> VanillaSample {
    let h = &params.hydrology;
    base.w *= 1.0 - clamp01(hydrology.river.max(local_river));
    base.e = lerp(base.e, h.floodplain_e, hydrology.floodplain);
    base.e = lerp(base.e, h.dry_basin_e, hydrology.dry_basin);
    base.e = lerp(base.e, h.swamp_e, hydrology.swamp);
    if hydrology.kind == SurfaceKind::InlandSea {
        // Глубина большого озера идёт через морской сплайн C, не через W.
        let depth_c = continentalness(-hydrology.lakes.sea_depth_km.max(0.0), 0.0, params);
        let target = clamp(depth_c, -1.0, h.inland_sea_c);
        base.c = lerp(base.c, target.min(base.c), hydrology.lakes.inland_sea);
    }
    if base.c > h.estuary_c {
        base.c = lerp(base.c, h.estuary_c, hydrology.estuary);
    }
    // Б7 не меняет C/E/W: только влажность и жаркая полоса температуры.
    base.h = clamp(base.h + h.gallery_h_gain * hydrology.gallery, -1.0, 1.0);
    if base.t > params.climate.t_centers[3] {
        base.t = lerp(base.t, params.climate.t_centers[3], hydrology.gallery);
    }
    base.c = clamp(base.c, -1.0, 1.0);
    base.e = clamp(base.e, -1.0, 1.0);
    base.w = clamp(base.w, -1.0, 1.0);
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ChannelHit;
    use crate::classify::classify_inputs;
    use crate::climate::{PoleSample, PrecipSample, SurfaceSample, TemperatureSample};
    use crate::lake::LakeHit;
    use crate::math::Vec2;
    use crate::mode::Mode;
    use crate::version::GenVersion;

    fn params() -> Params {
        Params::defaults(GenVersion::V1Dev, Mode::Gameplay)
    }

    fn climate(ratio: f64, mountain: f64, e_km: f64) -> ClimateSample {
        ClimateSample {
            point: Vec2::default(),
            surface: SurfaceSample {
                e_km,
                mountain,
                ..SurfaceSample::default()
            },
            pole: PoleSample {
                phi_deg: 40.0,
                north: Vec2::new(0.0, -1.0),
                east: Vec2::new(1.0, 0.0),
                pole: Vec2::default(),
                distance_l: 1.0,
            },
            wind: Vec2::new(1.0, 0.0),
            temperature: TemperatureSample {
                t_c: 15.0,
                sst_c: 15.0,
                amp_c: 8.0,
                t_bio: 15.0,
                t_min: 5.0,
            },
            precip: PrecipSample {
                mu: 0.5,
                cont: 0.5,
                p_mm: 1000.0,
                pet_mm: 1000.0 * ratio,
                ratio,
                slope: 0.0,
            },
        }
    }

    fn lake(kind: LakeKind, level: f64) -> LakeHit {
        LakeHit {
            plate_id: 1,
            lake: 0,
            depression: 0,
            kind,
            distance_l: 0.0,
            inside: true,
            level_km: Some(level),
            max_depth_km: 0.2,
            frozen: false,
            rift: false,
            high: level > 0.0,
            water_strength: 1.0,
            shore_strength: 1.0,
        }
    }

    fn lakes(hit: LakeHit) -> LakeQuery {
        let r1 = hit.level_km.is_some_and(|level| level <= 0.3);
        let mut q = LakeQuery {
            water: 1.0,
            shore: 1.0,
            basin: 1.0,
            nearest: Some(hit),
            water_hit: Some(hit),
            shore_hit: Some(hit),
            networks_considered: 1,
            ..LakeQuery::default()
        };
        if r1 {
            q.r1_water = 1.0;
        }
        match hit.kind {
            LakeKind::Swamp => {
                q.swamp = 1.0;
                if hit.high {
                    q.deferred_water = 1.0;
                }
            }
            LakeKind::DryBasin | LakeKind::ClosedSurplus => q.dry_basin = 1.0,
            LakeKind::InlandSea | LakeKind::SeaConnected if r1 => {
                q.inland_sea = 1.0;
                q.sea_depth_km = hit.max_depth_km;
            }
            LakeKind::Lake if r1 => q.river_lake = 1.0,
            _ => {
                q.deferred_water = 1.0;
                q.dry_basin = 1.0;
            }
        }
        q
    }

    fn base() -> VanillaSample {
        VanillaSample {
            c: 0.5,
            e: -0.2,
            w: 0.4,
            t: 0.0,
            h: 0.0,
        }
    }

    #[test]
    fn channel_core_and_bank_follow_distance_and_floodplain_avoids_mountains() {
        let p = params();
        let hit = ChannelHit {
            plate_id: 1,
            segment: 0,
            reach: 0,
            distance_l: 0.016,
            discharge_mm_l2: 100.0,
            core_l: 0.016,
            valley_l: 0.08,
            depth_blocks: 1.5,
            core_strength: 1.0,
            valley_strength: 0.8,
            delta_strength: 0.0,
            end: ReachEnd::Confluence,
            mouth_distance_l: 1.0,
        };
        let q = ChannelQuery {
            core: 1.0,
            valley: 0.8,
            nearest: Some(hit),
            networks_considered: 1,
            ..ChannelQuery::default()
        };
        let flat = resolve(q, LakeQuery::default(), &climate(2.0, 0.0, 0.1), &p);
        let mountain = resolve(q, LakeQuery::default(), &climate(2.0, 1.0, 2.0), &p);
        assert_eq!(flat.river, 1.0);
        assert!(flat.floodplain > 0.7 && mountain.floodplain == 0.0);
        let v = apply_fields(base(), 0.0, &flat, &p);
        assert_eq!(v.w, 0.0);
        assert!(v.e > p.hydrology.floodplain_e - 0.1);
        let mut bank = hit;
        let fade = (bank.core_l * p.hydrology.bank_fraction).max(p.hydrology.field_fade_l);
        bank.distance_l = bank.core_l + fade;
        assert_eq!(
            1.0 - smoothstep(bank.core_l, bank.core_l + fade, bank.distance_l),
            0.0
        );
    }

    #[test]
    fn small_lake_is_river_large_lake_is_ocean_and_swamp_is_e6() {
        let p = params();
        let c = climate(0.8, 0.0, 0.1);
        let small = resolve(
            ChannelQuery::default(),
            lakes(lake(LakeKind::Lake, 0.1)),
            &c,
            &p,
        );
        assert_eq!(small.kind, SurfaceKind::Lake);
        assert_eq!(small.river, 1.0);
        let big_hit = lake(LakeKind::InlandSea, 0.1);
        let big = resolve(ChannelQuery::default(), lakes(big_hit), &c, &p);
        let v = apply_fields(base(), 0.0, &big, &p);
        assert!(v.c < p.climate.ocean_c_threshold);
        assert_eq!(big.river, 0.0);
        let class = classify_surface(classify_inputs(15.0, 0.8, 5.0, &p), &big, &c, &p);
        assert!(class.is_ocean && class.cell.biome.contains("ocean"));
        let swamp = resolve(
            ChannelQuery::default(),
            lakes(lake(LakeKind::Swamp, 0.005)),
            &c,
            &p,
        );
        let v = apply_fields(base(), 0.0, &swamp, &p);
        assert_eq!(v.e, p.hydrology.swamp_e);
        assert!(v.c >= p.climate.ocean_c_threshold);
    }

    #[test]
    fn high_water_is_not_faked_at_sea_level_in_r1() {
        let p = params();
        let c = climate(2.0, 0.0, 1.0);
        let high = resolve(
            ChannelQuery::default(),
            lakes(lake(LakeKind::InlandSea, 0.8)),
            &c,
            &p,
        );
        assert_eq!(high.kind, SurfaceKind::DeferredWater);
        assert_eq!(high.dry_basin, 1.0);
        assert_eq!(high.water, 0.0);
        assert_eq!(high.river, 0.0);
        assert_eq!(high.r2_deferred, 1.0);
        let v = apply_fields(base(), 0.0, &high, &p);
        assert!(v.c >= p.climate.ocean_c_threshold);
        assert_eq!(v.e, p.hydrology.dry_basin_e);
    }

    #[test]
    fn frozen_lake_and_estuary_use_the_correct_biome_and_coast() {
        let p = params();
        let mut c = climate(0.8, 0.0, 0.02);
        c.temperature.t_min = -5.0;
        let mut hit = lake(LakeKind::Lake, 0.1);
        hit.frozen = true;
        let water = resolve(ChannelQuery::default(), lakes(hit), &c, &p);
        let class = classify_surface(classify_inputs(2.0, 0.8, -5.0, &p), &water, &c, &p);
        assert_eq!(class.cell.biome, "frozen_river");
        let mouth = ChannelHit {
            plate_id: 1,
            segment: 0,
            reach: 0,
            distance_l: 0.0,
            discharge_mm_l2: 2000.0,
            core_l: 0.02,
            valley_l: 0.1,
            depth_blocks: 2.0,
            core_strength: 1.0,
            valley_strength: 1.0,
            delta_strength: 0.0,
            end: ReachEnd::Sea,
            mouth_distance_l: 0.0,
        };
        let q = ChannelQuery {
            core: 1.0,
            valley: 1.0,
            nearest: Some(mouth),
            networks_considered: 1,
            ..ChannelQuery::default()
        };
        let estuary = resolve(q, LakeQuery::default(), &c, &p);
        assert_eq!(estuary.estuary, 1.0);
        let v = apply_fields(base(), 0.0, &estuary, &p);
        assert!((v.c - p.hydrology.estuary_c).abs() < 1e-12);
    }

    #[test]
    fn gallery_forest_needs_a_large_river_corridor_and_dry_land() {
        let p = params();
        let mut c = climate(5.0, 0.0, 0.2);
        c.temperature.t_bio = 30.0;
        c.temperature.t_min = 12.0;
        let corridor = ChannelQuery {
            gallery: 1.0,
            core: 0.0,
            valley: 0.0,
            ..ChannelQuery::default()
        };
        let h = resolve(corridor, LakeQuery::default(), &c, &p);
        assert_eq!(h.gallery, 1.0);
        let dry = classify_inputs(30.0, 5.0, 12.0, &p);
        assert_eq!(dry.cell.biome, "desert");
        let forest = classify_surface(dry, &h, &c, &p);
        assert_eq!(forest.cell.biome, "forest");
        assert_eq!(forest.cell.t, 3);
        let mut numbers = base();
        numbers.t = p.climate.t_centers[4];
        numbers.h = p.climate.h_centers[0];
        let v = apply_fields(numbers, 0.0, &h, &p);
        assert_eq!(v.t, p.climate.t_centers[3]);
        assert!((v.h - (numbers.h + p.hydrology.gallery_h_gain)).abs() < 1e-12);
        assert_eq!((v.c, v.e, v.w), (numbers.c, numbers.e, numbers.w));
        let wet = resolve(corridor, LakeQuery::default(), &climate(0.4, 0.0, 0.2), &p);
        let sea = resolve(corridor, LakeQuery::default(), &climate(5.0, 0.0, -0.1), &p);
        assert_eq!(wet.gallery, 0.0);
        assert_eq!(sea.gallery, 0.0);
        let edge = ChannelQuery {
            gallery: 0.25,
            ..ChannelQuery::default()
        };
        let edge = resolve(edge, LakeQuery::default(), &c, &p);
        assert_eq!(classify_surface(dry, &edge, &c, &p).cell.biome, "desert");
        assert!(apply_fields(numbers, 0.0, &edge, &p).h > numbers.h);
    }

    #[test]
    fn v0_formula_is_not_called_by_the_new_resolution_and_params_validate() {
        let mut p = params();
        assert!(validate(&p).is_ok());
        p.hydrology.bank_fraction = 0.0;
        assert!(matches!(
            validate(&p),
            Err(RiverNetError::InvalidParameters(_))
        ));
        let old = Params::defaults(GenVersion::V0, Mode::Gameplay);
        assert!(!old.canonical_text().contains("hydrology."));
    }
}
