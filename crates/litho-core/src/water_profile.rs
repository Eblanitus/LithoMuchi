//! Классы долин рек (`docs/PLAN-WATER-UPDATE.md` §5.2) и уклона/потока
//! (§5.3, §6.3 п. 7). Чистые функции без доступа к миру: их зовёт профиль
//! уровня реки (`generator.rs`, ВО1-1), вырез 6.5 (`height.rs`, ВО1-2) и
//! позже метрики 10.1. Braided (класс №4) появляется в ВО6, террасы в ВО5.

use crate::params::WaterUpdateParams;

/// Класс долины реки (§5.2). Порядок `table_index` совпадает с порядком
/// таблиц `floodplain_w` и `wall_slope` в `WaterUpdateParams` (§6.9):
/// [равнинная, холмистая, горная V, ледниковая U, каньон]; braided (5) —
/// с ВО6.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ValleyClass {
    #[default]
    Plain,
    Upland,
    MountainV,
    GlacialU,
    Canyon,
}

impl ValleyClass {
    pub const COUNT: usize = 5;

    /// Индекс в таблицах `floodplain_w` и `wall_slope` (§6.9).
    pub const fn table_index(self) -> usize {
        match self {
            Self::Plain => 0,
            Self::Upland => 1,
            Self::MountainV => 2,
            Self::GlacialU => 3,
            Self::Canyon => 4,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Upland => "upland",
            Self::MountainV => "mountain_v",
            Self::GlacialU => "glacial_u",
            Self::Canyon => "canyon",
        }
    }

    /// Тяжесть класса для разрешения ничьих при сглаживании: ничья уходит
    /// в пользу более мягкой долины.
    const fn severity(self) -> u8 {
        match self {
            Self::Plain => 0,
            Self::Upland => 1,
            Self::GlacialU => 2,
            Self::MountainV => 3,
            Self::Canyon => 4,
        }
    }

    const fn from_table_index(index: usize) -> Self {
        match index {
            0 => Self::Plain,
            1 => Self::Upland,
            2 => Self::MountainV,
            3 => Self::GlacialU,
            _ => Self::Canyon,
        }
    }
}

/// Вход классификации долины (§5.2); всё уже в шкале игры.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ValleyClassInput {
    /// Вес морфозоны 6 (холодные горы), 0…1.
    pub cold_mountain_weight: f64,
    /// Пояс P0–P1 (`t_c` < 3 °C, края §5.1).
    pub polar_belt: bool,
    /// Полоса E0–E1 (`e < height.e_knots[2]`).
    pub erosion_e0_e1: bool,
    /// `G` — глубина долины, блоки: `max(S0_macro)` на пробах `±R_v` от оси
    /// минус уровень `W` в точке (§6.3, п. 1).
    pub depth_blocks: f64,
    /// Сухость `R = PET/P`.
    pub aridity: f64,
    /// Вес плато, 0…1.
    pub plateau: f64,
    /// Уклон профиля `W` в точке (блок на блок).
    pub slope: f64,
}

/// §5.2: первое подходящее условие.
pub fn valley_class(input: ValleyClassInput, water: &WaterUpdateParams) -> ValleyClass {
    // №1 ледниковая U: холодные горы или полярный пояс в горах.
    if input.cold_mountain_weight >= 0.5 || (input.polar_belt && input.erosion_e0_e1) {
        return ValleyClass::GlacialU;
    }
    // №2 каньон: глубокий врез в сухом или на плато.
    if input.depth_blocks >= water.gorge_canyon_blocks
        && (input.aridity >= 1.5 || input.plateau >= 0.5)
    {
        return ValleyClass::Canyon;
    }
    // №3 горная V: крутой профиль или горы по эрозии.
    if input.slope >= 1.0 / 32.0 || input.erosion_e0_e1 {
        return ValleyClass::MountainV;
    }
    // №5 холмистая.
    if input.depth_blocks >= water.gorge_upland_blocks {
        return ValleyClass::Upland;
    }
    // №6 равнинная.
    ValleyClass::Plain
}

/// Класс уклона `W` (§5.3). Пороги — `water.slope_edges` (1/64, 1/16, 1/4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SlopeClass {
    #[default]
    Calm,
    Riffle,
    Rapid,
    Cascade,
}

pub fn slope_class(slope: f64, water: &WaterUpdateParams) -> SlopeClass {
    if slope < water.slope_edges[0] {
        SlopeClass::Calm
    } else if slope < water.slope_edges[1] {
        SlopeClass::Riffle
    } else if slope < water.slope_edges[2] {
        SlopeClass::Rapid
    } else {
        SlopeClass::Cascade
    }
}

/// Класс потока (§6.3, п. 7 / §6.8): классы уклона плюс водопад. В ВО1
/// водопад только размечается (`FallTop`), перепад раскладывается ступенями
/// по блоку, как в R2; котёд (`Plunge`) ставит вырез/Java (ВО5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlowClass {
    #[default]
    Calm,
    Riffle,
    Rapid,
    Cascade,
    /// Верхняя кромка водопада.
    FallTop,
    /// Колонка под водопадом (котёл). Профиль её не ставит.
    Plunge,
}

impl FlowClass {
    /// Биты 0–3 в `info_code()` (§6.8).
    pub const fn code(self) -> u32 {
        match self {
            Self::Calm => 0,
            Self::Riffle => 1,
            Self::Rapid => 2,
            Self::Cascade => 3,
            Self::FallTop => 4,
            Self::Plunge => 5,
        }
    }
}

/// Класс потока по падению `W` и локальному перепаду. `drop_blocks` —
/// перепад между соседними пробами вниз по течению. Водопад (п. 6) не
/// размечается, если выше по течению уже каскад.
pub fn flow_class(slope: f64, drop_blocks: f64, upstream: FlowClass, water: &WaterUpdateParams) -> FlowClass {
    if drop_blocks >= water.fall_min_drop_blocks && upstream != FlowClass::Cascade {
        return FlowClass::FallTop;
    }
    match slope_class(slope, water) {
        SlopeClass::Calm => FlowClass::Calm,
        SlopeClass::Riffle => FlowClass::Riffle,
        SlopeClass::Rapid => FlowClass::Rapid,
        SlopeClass::Cascade => FlowClass::Cascade,
    }
}

/// Классы на сегмент звена: ложатся в `PlateLevels` рядом с уровнями
/// (`generator.rs`, ВО1-1/ВО1-2).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SegmentClass {
    pub valley: ValleyClass,
    pub flow: FlowClass,
    /// `max(S0_macro)` на пробах `±R_v` от оси (§6.3, п. 1): по ней задаётся
    /// склон долины `s_eff` (§6.5). `0.0` — не считалась, склон = `s_wall`.
    pub gp: f64,
}

/// §5.2: класс долины сглаживается вдоль звена большинством голосов в окне
/// `window_blocks` (по оси, в блоках, окно центрировано). Ничья — в пользу
/// более мягкого класса.
pub fn smooth_valley_classes(
    along_blocks: &[f64],
    classes: &[ValleyClass],
    window_blocks: f64,
) -> Vec<ValleyClass> {
    let half = 0.5 * window_blocks.max(0.0);
    let mut out = Vec::with_capacity(classes.len());
    for i in 0..classes.len() {
        let center = along_blocks[i];
        let mut votes = [0_u32; ValleyClass::COUNT];
        for (j, class) in classes.iter().enumerate() {
            if (along_blocks[j] - center).abs() <= half {
                votes[class.table_index()] += 1;
            }
        }
        let mut best_votes = 0_u32;
        let mut best = classes[i];
        for (index, &current) in votes.iter().enumerate() {
            if current == 0 {
                continue;
            }
            let candidate = ValleyClass::from_table_index(index);
            if current > best_votes
                || (current == best_votes && candidate.severity() < best.severity())
            {
                best = candidate;
                best_votes = current;
            }
        }
        out.push(best);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::Mode;
    use crate::params::Params;

    fn water() -> WaterUpdateParams {
        Params::defaults(crate::version::GenVersion::V1DevW, Mode::Gameplay).water
    }

    fn input() -> ValleyClassInput {
        ValleyClassInput::default()
    }

    #[test]
    fn glacial_u_wins_by_weight_and_by_belt() {
        let w = water();
        let mut i = input();
        i.cold_mountain_weight = 0.5;
        assert_eq!(valley_class(i, &w), ValleyClass::GlacialU);
        let mut i = input();
        i.polar_belt = true;
        i.erosion_e0_e1 = true;
        assert_eq!(valley_class(i, &w), ValleyClass::GlacialU);
    }

    #[test]
    fn canyon_needs_depth_and_dryness_or_plateau() {
        let w = water();
        let mut i = input();
        i.depth_blocks = 30.0;
        i.aridity = 2.0;
        assert_eq!(valley_class(i, &w), ValleyClass::Canyon);
        let mut i = input();
        i.depth_blocks = 30.0;
        i.plateau = 0.6;
        assert_eq!(valley_class(i, &w), ValleyClass::Canyon);
        // Глубокая мокрая долина вне плато — не каньон.
        let mut i = input();
        i.depth_blocks = 30.0;
        i.aridity = 0.5;
        assert_ne!(valley_class(i, &w), ValleyClass::Canyon);
    }

    #[test]
    fn mountain_v_by_slope_or_erosion_band() {
        let w = water();
        let mut i = input();
        i.slope = 1.0 / 32.0;
        assert_eq!(valley_class(i, &w), ValleyClass::MountainV);
        let mut i = input();
        i.erosion_e0_e1 = true;
        assert_eq!(valley_class(i, &w), ValleyClass::MountainV);
    }

    #[test]
    fn upland_and_plain_by_depth() {
        let w = water();
        let mut i = input();
        i.depth_blocks = 5.0;
        assert_eq!(valley_class(i, &w), ValleyClass::Upland);
        let mut i = input();
        i.depth_blocks = 1.0;
        assert_eq!(valley_class(i, &w), ValleyClass::Plain);
    }

    #[test]
    fn flow_class_marks_falls_and_respects_cascade() {
        let w = water();
        assert_eq!(flow_class(0.5, 4.0, FlowClass::Calm, &w), FlowClass::FallTop);
        // Выше по течению каскад — водопад не размечается.
        assert_eq!(flow_class(0.5, 4.0, FlowClass::Cascade, &w), FlowClass::Cascade);
        assert_eq!(flow_class(0.001, 0.0, FlowClass::Calm, &w), FlowClass::Calm);
        assert_eq!(flow_class(0.02, 0.0, FlowClass::Calm, &w), FlowClass::Riffle);
        assert_eq!(flow_class(0.1, 0.0, FlowClass::Calm, &w), FlowClass::Rapid);
        assert_eq!(flow_class(0.3, 0.0, FlowClass::Calm, &w), FlowClass::Cascade);
    }

    #[test]
    fn smoothing_votes_window_and_breaks_ties_softly() {
        let along = [0.0, 10.0, 20.0, 30.0, 40.0];
        let classes = [
            ValleyClass::Canyon,
            ValleyClass::Plain,
            ValleyClass::Plain,
            ValleyClass::Canyon,
            ValleyClass::Plain,
        ];
        let out = smooth_valley_classes(&along, &classes, 64.0);
        // В окне ±32 блока вокруг каждой пробы — по 3 равнинные и 2 каньона,
        // поэтому везде равнинная.
        assert_eq!(out.iter().copied().collect::<Vec<_>>(), vec![ValleyClass::Plain; 5]);
        // Узкое окно (10) — каждая проба видит только себя и соседей на 5 бл.:
        // решения не смешиваются на весь массив.
        let out = smooth_valley_classes(&along, &classes, 10.0);
        assert_eq!(out[0], ValleyClass::Canyon);
        assert_eq!(out[2], ValleyClass::Plain);
    }
}
