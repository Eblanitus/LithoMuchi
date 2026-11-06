//! Версия генератора (план 6.8). Формулы выпущенной версии не меняются.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GenVersion {
    /// Рабочая версия: меняется свободно до первого выпуска.
    V0,
    /// Будущая замороженная версия первого выпуска. Пока параметры v0.
    V1,
}

impl GenVersion {
    pub const ALL: [Self; 2] = [Self::V0, Self::V1];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V0 => "v0",
            Self::V1 => "v1",
        }
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
    }
}
