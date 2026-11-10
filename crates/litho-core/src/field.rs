//! Поля, которые спрашивает игра, и полный результат в точке (план 9.4, T55).

use crate::classify::{ClassSample, LifeZone, VanillaCell};
use crate::climate::ClimateSample;
use crate::generator::WorldSample;
use crate::math::Vec2;
use crate::plates::PlateSample;
use crate::vanilla::VanillaSample;

/// Имена шумов в датапаке: `litospherity:v0/<режим>/<поле>` (план 5/8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    Continents,
    Erosion,
    Ridges,
    Temperature,
    Vegetation,
    Lava,
    /// T144 / полная I2: смещение рельефа из нашей высоты.
    Offset,
    /// T141 / I2-lite: собственный PV (`overworld/ridges_folded`), не f(W).
    Pv,
    /// T144: крутизна/фактор формы из нашей высоты.
    Factor,
    /// T144: острота пиков из нашей высоты.
    Jaggedness,
}

impl Field {
    pub const ALL: [Self; 10] = [
        Self::Continents,
        Self::Erosion,
        Self::Ridges,
        Self::Temperature,
        Self::Vegetation,
        Self::Lava,
        Self::Offset,
        Self::Pv,
        Self::Factor,
        Self::Jaggedness,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Continents => "continents",
            Self::Erosion => "erosion",
            Self::Ridges => "ridges",
            Self::Temperature => "temperature",
            Self::Vegetation => "vegetation",
            Self::Lava => "lava",
            Self::Offset => "offset",
            Self::Pv => "pv",
            Self::Factor => "factor",
            Self::Jaggedness => "jaggedness",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|f| f.as_str() == text)
    }
}

/// Всё, что знает генератор о точке. `world` — то же самое, что даёт
/// эталонный путь, поэтому слои песочницы и мод видят одинаковые поля.
#[derive(Clone, Debug)]
pub struct FullSample {
    pub point: Vec2,
    pub plate: Option<PlateSample>,
    pub world: WorldSample,
    pub lava: f64,
    pub zone: LifeZone,
    pub cell: VanillaCell,
    pub climate: ClimateSample,
    pub class: ClassSample,
    pub vanilla: VanillaSample,
}

/// Перевод в `f32` делается только на границе с Horizon (9.4). Нечисловое
/// значение становится нулём, а не NaN: генерация не должна ронять игру.
pub fn to_f32(value: f64) -> f32 {
    if value.is_finite() { value as f32 } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::{Field, to_f32};

    #[test]
    fn all_field_names_round_trip() {
        for field in Field::ALL {
            assert_eq!(Field::parse(field.as_str()), Some(field));
        }
        assert_eq!(Field::parse("unknown"), None);
    }

    #[test]
    fn non_finite_values_become_zero() {
        assert_eq!(to_f32(f64::NAN), 0.0);
        assert_eq!(to_f32(f64::INFINITY), 0.0);
        assert_eq!(to_f32(0.25), 0.25_f32);
    }
}
