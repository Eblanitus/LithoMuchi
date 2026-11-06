//! Точка входа в ядро (эскиз публичного API 9.4).
//!
//! Сейчас здесь собрано то, что уже реализовано: примитивы, плиты, искажение
//! домена. Полный `FullSample` (рельеф, климат, биомы, C/E/W/T/H) появится в
//! T55, но песочница и мод уже вызывают один и тот же `Generator`.

use crate::math::Vec2;
use crate::mode::Mode;
use crate::params::Params;
use crate::plates::{PlateSample, plate_query, warp_jacobian};
use crate::seed::from_world_seed;
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

#[derive(Clone, Debug)]
pub struct Generator {
    seed: u64,
    params: Params,
}

impl Generator {
    /// Создаётся один раз на мир. Пока дёшево; после T54 здесь появится поиск
    /// сдвига точки появления `p*` (около 256 эталонных расчётов).
    pub fn new(seed: i64, mode: Mode, version: GenVersion) -> Result<Self, GenError> {
        if version != GenVersion::V0 {
            return Err(GenError::UnsupportedVersion(version));
        }
        Ok(Self {
            seed: from_world_seed(seed),
            params: Params::defaults(version, mode),
        })
    }

    pub const fn seed(&self) -> u64 {
        self.seed
    }

    pub const fn params(&self) -> &Params {
        &self.params
    }

    /// Сдвиг макро-координат `p*` (7.12). До T54 равен нулю.
    pub fn spawn_shift(&self) -> Vec2 {
        Vec2::default()
    }

    /// Блочные координаты → макро-координата `p = (x/L, z/L) + p*`.
    pub fn macro_point(&self, x: f64, z: f64) -> Vec2 {
        let l = self.params.block.l_blocks;
        Vec2::new(x / l, z / l).add(self.spawn_shift())
    }

    /// Плита и границы в блочной точке.
    pub fn plate_sample(&self, x: f64, z: f64) -> Option<PlateSample> {
        plate_query(self.macro_point(x, z), self.seed, &self.params)
    }

    /// Диагностика искажения домена (слой `jacobian`).
    pub fn jacobian(&self, x: f64, z: f64) -> f64 {
        warp_jacobian(self.macro_point(x, z), self.seed, &self.params)
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
    fn macro_coordinates_scale_with_mode() {
        let sci = Generator::new(42, Mode::Scientific, GenVersion::V0).unwrap();
        let game = Generator::new(42, Mode::Gameplay, GenVersion::V0).unwrap();
        let a = sci.macro_point(4000.0, -2000.0);
        let b = game.macro_point(1000.0, -500.0);
        assert_eq!(a, b);
    }
}
