//! Точка входа в ядро (эскиз публичного API 9.4).
//!
//! Здесь собраны примитивы, плиты, кора, климат, классификация и пять
//! ванильных параметров. Полный быстрый путь с решёткой и кэшем появится в
//! T13/T55, но песочница и будущий мод уже вызывают один и тот же `Generator`.

use std::sync::Arc;

use crate::classify::{ClassSample, classify};
use crate::climate::{ClimateSample, SurfaceSample};
use crate::crust::{CrustSample, crust_from_sample};
use crate::math::Vec2;
use crate::mode::Mode;
use crate::params::Params;
use crate::plates::{PlateSample, plate_query, warp_jacobian};
use crate::rivers::{RiverCache, local_r, traced_r};
use crate::seed::from_world_seed;
use crate::spawn::{self, SpawnQuality};
use crate::vanilla::{VanillaSample, continentalness};
use crate::version::GenVersion;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenError {
    /// Версия ещё не реализована (v1 появится при заморозке T90).
    UnsupportedVersion(GenVersion),
}

impl std::fmt::Display for GenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion(v) => write!(f, "версия генератора {v:?} не реализована"),
        }
    }
}

impl std::error::Error for GenError {}

/// Всё, что известно о точке мира после слоёв L2, L3, L5 и L6 (T30–T43).
#[derive(Clone, Debug)]
pub struct WorldSample {
    pub climate: ClimateSample,
    pub class: ClassSample,
    pub vanilla: VanillaSample,
    /// Сила рифтовой/предгорной реки, вошедшая в W (7.9.3).
    pub river: f64,
    pub is_ocean: bool,
}

#[derive(Clone, Debug)]
pub struct Generator {
    seed: u64,
    params: Params,
    rivers: Arc<RiverCache>,
    spawn_shift: Vec2,
}

impl Generator {
    /// Создаётся один раз на мир. Пока дёшево; после T54 здесь появится поиск
    /// сдвига точки появления `p*` (около 256 эталонных расчётов).
    pub fn new(seed: i64, mode: Mode, version: GenVersion) -> Result<Self, GenError> {
        if version != GenVersion::V0 {
            return Err(GenError::UnsupportedVersion(version));
        }
        let world_seed = from_world_seed(seed);
        let params = Params::defaults(version, mode);
        // T54: точка появления ищется один раз на мир, только по сиду и версии.
        let shift = spawn::find_shift(world_seed, &params);
        Ok(Self {
            seed: world_seed,
            params,
            rivers: Arc::new(RiverCache::new()),
            spawn_shift: shift,
        })
    }

    pub const fn seed(&self) -> u64 {
        self.seed
    }

    pub const fn params(&self) -> &Params {
        &self.params
    }

    /// Сдвиг макро-координат `p*` (7.12). В v0 найден и доступен слою `spawn`,
    /// но ещё не подставлен в `macro_point`: карты песочницы остаются на месте
    /// до T55, чтобы автор сравнивал их с прежними (Д-015).
    pub fn spawn_shift(&self) -> Vec2 {
        self.spawn_shift
    }

    /// Блочные координаты → макро-координата `p = x/L, z/L`.
    /// Сдвиг `p*` (7.12) в v0 найден, но не подставлен сюда (Д-015): карты
    /// песочницы остаются на прежнем месте планеты, слой `spawn` центрируется
    /// на `p*` сам.
    pub fn macro_point(&self, x: f64, z: f64) -> Vec2 {
        let l = self.params.block.l_blocks;
        Vec2::new(x / l, z / l)
    }

    /// Плита и границы в блочной точке.
    pub fn plate_sample(&self, x: f64, z: f64) -> Option<PlateSample> {
        plate_query(self.macro_point(x, z), self.seed, &self.params)
    }

    /// Кора и макрорельеф в блочной точке (T22–T24).
    pub fn crust_sample(&self, x: f64, z: f64) -> Option<CrustSample> {
        let point = self.macro_point(x, z);
        let sample = plate_query(point, self.seed, &self.params)?;
        Some(crust_from_sample(point, &sample, self.seed, &self.params))
    }

    /// Диагностика искажения домена (слой `jacobian`).
    pub fn jacobian(&self, x: f64, z: f64) -> f64 {
        warp_jacobian(self.macro_point(x, z), self.seed, &self.params)
    }

    /// Поверхность в произвольной macro-точке: нужно линии тока влаги (7.7).
    fn surface_at(&self, point: Vec2) -> SurfaceSample {
        match plate_query(point, self.seed, &self.params) {
            Some(sample) => {
                let crust = crust_from_sample(point, &sample, self.seed, &self.params);
                SurfaceSample {
                    e_km: crust.e_km,
                    iota_l: crust.iota_l,
                    mountain: crust.mountain,
                    plateau: crust.plateau,
                    boundary_weights: crust.boundary_weights,
                    sigma_l: sample.d_ab_l,
                }
            }
            None => SurfaceSample::default(),
        }
    }

    /// Климат в блочной точке (T30–T33).
    pub fn climate_sample(&self, x: f64, z: f64) -> Option<ClimateSample> {
        let point = self.macro_point(x, z);
        crate::climate::sample(point, self.seed, &self.params, &|q| self.surface_at(q))
    }

    /// Климат, жизненная зона и пять ванильных чисел (T40–T43).
    pub fn world_sample(&self, x: f64, z: f64) -> Option<WorldSample> {
        let climate = self.climate_sample(x, z)?;
        let c = continentalness(climate.surface.e_km, climate.surface.iota_l, &self.params);
        let is_ocean = c < self.params.climate.ocean_c_threshold;
        let class = classify(&climate, is_ocean, x, z, self.seed, &self.params);
        // Дешёвая часть рек входит в W везде; трассированные реки считаются
        // отдельно для слоя `rivers` (Д-015).
        let river = local_r(x, z, &climate.surface, self.seed, &self.params);
        let vanilla =
            crate::vanilla::sample(&climate, &class, x, z, self.seed, &self.params, river);
        Some(WorldSample {
            climate,
            class,
            vanilla,
            river,
            is_ocean,
        })
    }

    /// Полная сила реки вместе с трассированными руслами (слой `rivers`).
    pub fn river_sample(&self, x: f64, z: f64) -> f64 {
        let point = self.macro_point(x, z);
        let surface = self.surface_at(point);
        let local = local_r(x, z, &surface, self.seed, &self.params);
        let traced = traced_r(
            point,
            surface.e_km,
            self.seed,
            &self.params,
            &|q| self.surface_at(q),
            &self.rivers,
        );
        local.max(traced)
    }

    /// Поле глубинной лавы λ (слой `lava`, T53).
    pub fn lava_sample(&self, x: f64, z: f64) -> f64 {
        let point = self.macro_point(x, z);
        match plate_query(point, self.seed, &self.params) {
            Some(sample) => crate::lava::sample(point, &sample, self.seed, &self.params),
            None => self.params.lava.background,
        }
    }

    /// Насколько точка подходит для появления (слой `spawn`, T54).
    pub fn spawn_quality(&self, x: f64, z: f64) -> SpawnQuality {
        spawn::evaluate(self.macro_point(x, z), self.seed, &self.params)
    }
}

#[cfg(test)]
mod tests {
    use super::{GenError, Generator};
    use crate::mode::Mode;
    use crate::version::GenVersion;

    #[test]
    fn v1_is_not_implemented_yet() {
        assert_eq!(
            Generator::new(1, Mode::Gameplay, GenVersion::V1).unwrap_err(),
            GenError::UnsupportedVersion(GenVersion::V1)
        );
    }

    #[test]
    fn world_sample_is_finite_and_bounded() {
        let generator = Generator::new(42, Mode::Gameplay, GenVersion::V0).unwrap();
        for k in 0..12 {
            let x = k as f64 * 137.0 - 800.0;
            let z = k as f64 * -83.0 + 400.0;
            let w = generator.world_sample(x, z).unwrap();
            assert!(w.climate.temperature.t_bio.is_finite());
            assert!(w.climate.precip.p_mm.is_finite() && w.climate.precip.p_mm >= 0.0);
            assert!(w.climate.precip.ratio.is_finite() && w.climate.precip.ratio > 0.0);
            for value in [
                w.vanilla.c,
                w.vanilla.e,
                w.vanilla.w,
                w.vanilla.t,
                w.vanilla.h,
            ] {
                assert!(
                    (-1.0..=1.0).contains(&value),
                    "значение {value} вне [-1, 1]"
                );
            }
        }
    }

    #[test]
    fn macro_coordinates_scale_with_mode() {
        let sci = Generator::new(42, Mode::Scientific, GenVersion::V0).unwrap();
        let game = Generator::new(42, Mode::Gameplay, GenVersion::V0).unwrap();
        let a = sci.macro_point(4000.0, -2000.0);
        let b = game.macro_point(1000.0, -500.0);
        assert_eq!(a, b);
    }
}
