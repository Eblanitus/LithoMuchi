//! Версия генератора (план 6.8). Формулы выпущенной версии не меняются.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GenVersion {
    /// Рабочая версия: меняется свободно до первого выпуска.
    V0,
    /// Будущая замороженная версия первого выпуска. Пока параметры v0.
    V1,
    /// Э3.5: рабочая версия «Вода и рельеф», не совместима с v0.
    /// Не является выпущенной V1 и меняется до заморозки T151/T90.
    V1Dev,
    /// R2-Java (Ф6-В1, PLAN-WATER-FABRIC): вода выше уровня моря на Fabric.
    /// Отличается от `V1Dev` только водой: предел `r1_max_level_km` снят,
    /// уровень берётся от сухой поверхности колонки (озеро — `min(S)` по
    /// контуру минус 1, русло — низший берег минус 1), вырез только вниз,
    /// берег добивается на 1–2 блока (Д-123, Д-125).
    /// Миры Horizon её не создают; паритет Ф4 гоняется на `V1Dev`.
    V1DevR2,
}

impl GenVersion {
    pub const ALL: [Self; 4] = [Self::V0, Self::V1, Self::V1Dev, Self::V1DevR2];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V0 => "v0",
            Self::V1 => "v1",
            Self::V1Dev => "v1-dev",
            Self::V1DevR2 => "v1-dev-r2",
        }
    }

    /// Амплитуды в σ-единицах и узкий знак W применяются только здесь.
    /// Ветку нельзя включать для v0: его формулы заморожены (Д-026).
    pub const fn is_water_relief_dev(self) -> bool {
        matches!(self, Self::V1Dev | Self::V1DevR2)
    }

    /// R2 (Ф6-В1): вода стоит и выше уровня моря. Только эта версия меняет
    /// `with_water` и снимает предел `hydrology.r1_max_level_km`.
    pub const fn is_water_relief_r2(self) -> bool {
        matches!(self, Self::V1DevR2)
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|v| v.as_str() == text)
    }
}

#[cfg(test)]
mod tests {
    use super::GenVersion;

    #[test]
    fn parse_roundtrip() {
        for version in GenVersion::ALL {
            assert_eq!(GenVersion::parse(version.as_str()), Some(version));
        }
        assert_eq!(GenVersion::parse("v9"), None);
        assert!(!GenVersion::V0.is_water_relief_dev());
        assert!(!GenVersion::V1.is_water_relief_dev());
        assert!(GenVersion::V1Dev.is_water_relief_dev());
        assert!(GenVersion::V1DevR2.is_water_relief_dev());
        assert!(!GenVersion::V1Dev.is_water_relief_r2());
        assert!(GenVersion::V1DevR2.is_water_relief_r2());
        // Индекс в `litho_generator_new` (Ф6-В2): ABI не меняется, R2 — 3.
        assert_eq!(
            GenVersion::ALL
                .iter()
                .position(|v| *v == GenVersion::V1DevR2),
            Some(3)
        );
    }
}
