//! Два режима одной планеты (план 8).
//!
//! Все макро-величины заданы в `L` и у режимов одинаковы; различаются только
//! `L` в блоках и блочные детали (`BlockParams`). Геймплейный режим — та же
//! планета, уменьшенная в 4 раза: макро-поля в `(x, z)` равны научным в
//! `(4x, 4z)`.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Scientific,
    Gameplay,
}

impl Mode {
    pub const ALL: [Self; 2] = [Self::Scientific, Self::Gameplay];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scientific => "scientific",
            Self::Gameplay => "gameplay",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|m| m.as_str() == text)
    }

    /// Число блоков в одной макро-единице L.
    pub const fn l_blocks(self) -> f64 {
        match self {
            Self::Scientific => 1000.0,
            Self::Gameplay => 250.0,
        }
    }

    /// Блочные параметры режима (таблица 8.3).
    pub const fn block_params(self) -> BlockParams {
        match self {
            Self::Scientific => BlockParams {
                l_blocks: 1000.0,
                delta_fine_blocks: 62.5,
                delta_coarse_blocks: 250.0,
                lambda_e_blocks: 1200.0,
                lambda_w_blocks: 500.0,
                lambda_s_blocks: 6000.0,
                river_half_width_blocks: 24.0,
                meander_amplitude_blocks: 80.0,
                meander_lambda_blocks: 600.0,
                jitter_t_c: 0.8,
                jitter_r_log2: 0.25,
                lava_width_blocks: 300.0,
                lava_background: 0.05,
                swamps_enabled: true,
                mushroom_islands_enabled: false,
            },
            Self::Gameplay => BlockParams {
                l_blocks: 250.0,
                delta_fine_blocks: 15.625,
                delta_coarse_blocks: 62.5,
                lambda_e_blocks: 600.0,
                lambda_w_blocks: 300.0,
                lambda_s_blocks: 1600.0,
                river_half_width_blocks: 14.0,
                meander_amplitude_blocks: 50.0,
                meander_lambda_blocks: 350.0,
                jitter_t_c: 1.5,
                jitter_r_log2: 0.5,
                lava_width_blocks: 120.0,
                lava_background: 0.15,
                swamps_enabled: true,
                mushroom_islands_enabled: false,
            },
        }
    }
}

/// Параметры, которые НЕ масштабируются вместе с L (раздел 8.3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockParams {
    pub l_blocks: f64,
    pub delta_fine_blocks: f64,
    pub delta_coarse_blocks: f64,
    pub lambda_e_blocks: f64,
    pub lambda_w_blocks: f64,
    pub lambda_s_blocks: f64,
    pub river_half_width_blocks: f64,
    pub meander_amplitude_blocks: f64,
    pub meander_lambda_blocks: f64,
    pub jitter_t_c: f64,
    pub jitter_r_log2: f64,
    pub lava_width_blocks: f64,
    pub lava_background: f64,
    pub swamps_enabled: bool,
    pub mushroom_islands_enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::Mode;

    #[test]
    fn parse_roundtrip() {
        for mode in Mode::ALL {
            assert_eq!(Mode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(Mode::parse("creative"), None);
    }

    #[test]
    fn one_planet_two_scales() {
        assert_eq!(Mode::Scientific.l_blocks(), 4.0 * Mode::Gameplay.l_blocks());
        assert_eq!(
            Mode::Scientific.block_params().delta_fine_blocks,
            4.0 * Mode::Gameplay.block_params().delta_fine_blocks
        );
    }
}
