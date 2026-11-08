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
}

impl GenVersion {
    pub const ALL: [Self; 3] = [Self::V0, Self::V1, Self::V1Dev];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V0 => "v0",
            Self::V1 => "v1",
            Self::V1Dev => "v1-dev",
        }
    }

    /// Амплитуды в σ-единицах и узкий знак W применяются только здесь.
    /// Ветку нельзя включать для v0: его формулы заморожены (Д-026).
    pub const fn is_water_relief_dev(self) -> bool {
        matches!(self, Self::V1Dev)
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
        assert!(GenVersion::V1Dev.is_water_relief_dev());
        assert!(!GenVersion::V0.is_water_relief_dev());
        assert!(!GenVersion::V1.is_water_relief_dev());
    }
}
