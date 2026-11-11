//! T151/10.1: вердикт автоматических критериев границ биомов.
//!
//! `stats`/`stats-all` печатают числа, но не говорят «пройдено/не пройдено» и
//! не умеют считать приёмку по частям. Полный скан 16 сидов × 2 режима при
//! ≥200×200 L — это миллионы точек на прогон, поэтому нужен инструмент,
//! который (а) берёт подмножество сидов и режимов, (б) сравнивает числа с
//! целями 10.1 и (в) честно помечает объём, если он меньше приёмочного.
//!
//! Пороги здесь — цитата целей 10.1, а не новые числа генератора: они не
//! входят в `params_hash` и не меняют мир.

use crate::height_stats::{BoundaryMetrics, HeightStats};

/// Цель 10.1: скачок h≥2 в пределах 32 блоков, доля точек суши.
pub const TARGET_JUMP_CLIMATE_PCT: f64 = 2.0;
/// Цель 10.1: поворот ветра не больше 5° на 32 блока.
pub const TARGET_WIND_DEG: f64 = 5.0;
/// Цель 10.1: доля длины границ в прямых отрезках >512 блоков, отклонение <16.
pub const TARGET_STRAIGHT_PCT: f64 = 1.0;
/// Цель 10.1: полосок биомов уже 32 блоков нет.
pub const TARGET_THIN_PCT: f64 = 0.0;
/// Допуск на квантование сетки: при сетке с шагом <32 блока одна полоска уже
/// 32 блоков может разрешаться в ширину 31.99 и не должна давать FAIL.
pub const THIN_TOLERANCE_PCT: f64 = 0.05;

/// Режим вердикта: цель 10.1 и допуск на округление конкретной сетки.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
    /// Число есть, но объём скана меньше приёмочного: это не приёмка 10.1.
    NotAcceptance,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::NotAcceptance => "NO-VOLUME",
        }
    }
    pub fn passed(self) -> bool {
        self == Self::Pass
    }
}

/// Одна проверка: значение, цель и вердикт.
#[derive(Clone, Copy, Debug)]
pub struct Check {
    pub name: &'static str,
    pub value: f64,
    /// Цель 10.1: проверка всегда «значение не больше цели».
    pub target: f64,
    pub verdict: Verdict,
}

impl Check {
    /// Все цели 10.1 — верхние границы, поэтому отдельного флага направления нет.
    fn upper(name: &'static str, value: f64, target: f64, tolerance: f64, volume: bool) -> Self {
        let ok = value.is_finite() && value <= target + tolerance;
        Self::with(name, value, target, ok, volume)
    }
    fn with(name: &'static str, value: f64, target: f64, ok: bool, volume: bool) -> Self {
        let verdict = if !volume {
            Verdict::NotAcceptance
        } else if ok {
            Verdict::Pass
        } else {
            Verdict::Fail
        };
        Self {
            name,
            value,
            target,
            verdict,
        }
    }
    pub fn text(&self) -> String {
        format!(
            "{:<26} {:>8.3} ≤ {:>6.2}  {}",
            self.name,
            self.value,
            self.target,
            self.verdict.as_str()
        )
    }
}

/// Итог по одному прогону (сид + режим).
#[derive(Clone, Debug)]
pub struct RunVerdict {
    pub seed: i64,
    pub mode: &'static str,
    pub checks: Vec<Check>,
    /// Объём сетки границ достаточен для 10.1.
    pub acceptance_volume: bool,
}

impl RunVerdict {
    pub fn from_stats(stats: &HeightStats) -> Self {
        let b: &BoundaryMetrics = &stats.boundary;
        let volume = b.acceptance_volume();
        // Только цели 10.1. Скачок по всей суше остаётся диагностикой `stats`:
        // в 10.1 для него цели нет, и выдумывать порог здесь нельзя.
        let checks = vec![
            Check::upper(
                "скачок h≥2, климат",
                b.climate_humidity_jump_pct(),
                TARGET_JUMP_CLIMATE_PCT,
                0.0,
                volume,
            ),
            Check::upper(
                "ветер, °/32 блока",
                b.wind_max_deg,
                TARGET_WIND_DEG,
                0.0,
                volume,
            ),
            Check::upper(
                "прямолинейность",
                b.straight_pct(),
                TARGET_STRAIGHT_PCT,
                0.0,
                volume,
            ),
            Check::upper(
                "полоски <32",
                b.thin_pct(),
                TARGET_THIN_PCT,
                THIN_TOLERANCE_PCT,
                volume,
            ),
        ];
        Self {
            seed: stats.seed,
            mode: stats.mode.as_str(),
            checks,
            acceptance_volume: volume,
        }
    }
    pub fn failed(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| c.verdict == Verdict::Fail)
            .count()
    }
    pub fn pending(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| c.verdict == Verdict::NotAcceptance)
            .count()
    }
    /// Вердикт по прогону: PASS только если объём приёмочный и все проверки прошли.
    pub fn verdict(&self) -> Verdict {
        if self.pending() > 0 {
            Verdict::NotAcceptance
        } else if self.failed() == 0 {
            Verdict::Pass
        } else {
            Verdict::Fail
        }
    }
    pub fn text(&self) -> String {
        let mut out = format!(
            "сид={} режим={} объём={} итог={}\n",
            self.seed,
            self.mode,
            if self.acceptance_volume {
                "приёмочный"
            } else {
                "МАЛЫЙ (не приёмка)"
            },
            self.verdict().as_str()
        );
        for check in &self.checks {
            out.push_str("  ");
            out.push_str(&check.text());
            out.push('\n');
        }
        out
    }
    pub fn json(&self) -> String {
        let checks = self
            .checks
            .iter()
            .map(|c| {
                format!(
                    "{{\"name\": {}, \"value\": {:.6}, \
                     \"target\": {:.6}, \"verdict\": {}}}",
                    crate::json::quote(c.name),
                    c.value,
                    c.target,
                    crate::json::quote(c.verdict.as_str())
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{{\"seed\": {}, \"mode\": {}, \"acceptance_volume\": {}, \
             \"verdict\": {}, \"checks\": [{}]}}",
            self.seed,
            crate::json::quote(self.mode),
            self.acceptance_volume,
            crate::json::quote(self.verdict().as_str()),
            checks
        )
    }
}

/// Сводка по пачке прогонов.
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub runs: Vec<RunVerdict>,
}

impl Summary {
    pub fn push(&mut self, run: RunVerdict) {
        self.runs.push(run);
    }
    pub fn passed(&self) -> usize {
        self.runs.iter().filter(|r| r.verdict().passed()).count()
    }
    pub fn failed(&self) -> usize {
        self.runs
            .iter()
            .filter(|r| r.verdict() == Verdict::Fail)
            .count()
    }
    pub fn pending(&self) -> usize {
        self.runs
            .iter()
            .filter(|r| r.verdict() == Verdict::NotAcceptance)
            .count()
    }
    /// Полный объём 10.1: 16 сидов × 2 режима, все приёмочные.
    pub fn is_full_volume(&self) -> bool {
        self.runs.len() == 32 && self.runs.iter().all(|r| r.acceptance_volume)
    }
    /// Общий вердикт 10.1.1 по границам. `false`, пока есть хоть один FAIL или
    /// пока скан неполный: неполный скан не доказывает выполнение цели.
    pub fn verdict(&self) -> Verdict {
        if self.failed() > 0 {
            Verdict::Fail
        } else if !self.is_full_volume() {
            Verdict::NotAcceptance
        } else {
            Verdict::Pass
        }
    }
    pub fn text(&self) -> String {
        let mut out = String::new();
        for run in &self.runs {
            out.push_str(&run.text());
        }
        out.push_str(&format!(
            "ИТОГ 10.1 (границы): прогонов {}, PASS {}, FAIL {}, без объёма {}; \
             полный объём 16 сидов × 2 режима: {}; вердикт {}\n",
            self.runs.len(),
            self.passed(),
            self.failed(),
            self.pending(),
            if self.is_full_volume() {
                "да"
            } else {
                "НЕТ"
            },
            self.verdict().as_str()
        ));
        out
    }
    pub fn json(&self) -> String {
        format!(
            "{{\"runs\": [{}], \"passed\": {}, \"failed\": {}, \"pending\": {}, \
             \"full_volume\": {}, \"verdict\": {}}}",
            self.runs
                .iter()
                .map(RunVerdict::json)
                .collect::<Vec<_>>()
                .join(", "),
            self.passed(),
            self.failed(),
            self.pending(),
            self.is_full_volume(),
            crate::json::quote(self.verdict().as_str())
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::height_stats::BoundaryMetrics;

    /// Носитель без скана: большая сетка границ (объём приёмки) и заданные
    /// счётчики. Размер только объявляет объём, точек не считает.
    fn stats(seed: i64, boundary: BoundaryMetrics) -> HeightStats {
        let mut s = HeightStats::empty_for_tests(4096, 32);
        s.seed = seed;
        s.boundary = boundary;
        s
    }

    fn small(seed: i64, climate_jump: f64) -> HeightStats {
        let mut b = BoundaryMetrics::empty(64, 32.0, 250.0);
        b.climate_land = 10_000;
        b.climate_humidity_jump_land = (climate_jump * 100.0) as usize;
        stats(seed, b)
    }

    fn full(seed: i64, climate_jump: f64) -> HeightStats {
        let mut b = BoundaryMetrics::empty(4096, 32.0, 250.0);
        b.climate_land = 10_000;
        b.climate_humidity_jump_land = (climate_jump * 100.0) as usize;
        b.land = 10_000;
        b.wind_max_deg = 1.0;
        stats(seed, b)
    }

    #[test]
    fn acceptance_targets_match_section_10_1() {
        assert_eq!(TARGET_JUMP_CLIMATE_PCT, 2.0);
        assert_eq!(TARGET_WIND_DEG, 5.0);
        assert_eq!(TARGET_STRAIGHT_PCT, 1.0);
        assert_eq!(TARGET_THIN_PCT, 0.0);
    }

    #[test]
    fn small_scan_is_not_acceptance_even_when_numbers_are_good() {
        let run = RunVerdict::from_stats(&small(42, 0.5));
        assert!(!run.acceptance_volume);
        assert_eq!(run.verdict(), Verdict::NotAcceptance);
        assert_eq!(run.failed(), 0);
        assert!(run.pending() > 0);
    }

    #[test]
    fn full_volume_with_bad_climate_jump_fails() {
        let run = RunVerdict::from_stats(&full(42, 5.0));
        assert!(run.acceptance_volume);
        assert_eq!(run.verdict(), Verdict::Fail);
        assert_eq!(run.failed(), 1);
    }

    #[test]
    fn full_volume_with_good_numbers_passes() {
        let run = RunVerdict::from_stats(&full(42, 0.5));
        assert_eq!(run.verdict(), Verdict::Pass);
    }

    #[test]
    fn summary_needs_all_32_runs_for_full_volume() {
        let mut summary = Summary::default();
        for seed in 0..31 {
            summary.push(RunVerdict::from_stats(&full(seed, 0.5)));
        }
        assert_eq!(summary.verdict(), Verdict::NotAcceptance);
        summary.push(RunVerdict::from_stats(&full(31, 0.5)));
        assert!(summary.is_full_volume());
        assert_eq!(summary.verdict(), Verdict::Pass);
        assert_eq!(summary.passed(), 32);
    }

    #[test]
    fn one_failed_run_makes_summary_fail() {
        let mut summary = Summary::default();
        for seed in 0..31 {
            summary.push(RunVerdict::from_stats(&full(seed, 0.5)));
        }
        summary.push(RunVerdict::from_stats(&full(99, 9.0)));
        assert_eq!(summary.verdict(), Verdict::Fail);
        assert_eq!(summary.failed(), 1);
    }

    #[test]
    fn hydrology_jump_without_target_does_not_change_verdict() {
        // Скачок по всей суше — диагностика, а не цель 10.1: он не должен
        // переводить прогон в FAIL сам по себе.
        let mut s = full(42, 0.0);
        s.boundary.land = 10_000;
        s.boundary.humidity_jump_land = 9_000;
        s.boundary.humidity_pairs = 10_000;
        s.boundary.humidity_jump_pairs = 9_000;
        let run = RunVerdict::from_stats(&s);
        assert_eq!(run.verdict(), Verdict::Pass);
    }

    #[test]
    fn thin_strips_fail_above_tolerance() {
        let mut s = full(42, 0.5);
        s.boundary.land = 10_000;
        s.boundary.thin_land = 500;
        let run = RunVerdict::from_stats(&s);
        assert_eq!(run.verdict(), Verdict::Fail);
    }

    #[test]
    fn wind_above_five_degrees_fails() {
        let mut s = full(42, 0.5);
        s.boundary.wind_max_deg = 6.5;
        let run = RunVerdict::from_stats(&s);
        assert_eq!(run.verdict(), Verdict::Fail);
    }

    #[test]
    fn nan_is_a_failure_not_a_pass() {
        let mut s = full(42, 0.5);
        s.boundary.wind_max_deg = f64::NAN;
        let run = RunVerdict::from_stats(&s);
        assert_eq!(run.verdict(), Verdict::Fail);
    }

    #[test]
    fn json_verdict_is_parseable() {
        let run = RunVerdict::from_stats(&full(42, 0.5));
        let value = crate::json::parse(&run.json()).unwrap();
        assert_eq!(
            value.get("verdict"),
            Some(&crate::json::Json::String("PASS".into()))
        );
        let checks = value.get("checks").and_then(|c| c.as_array()).unwrap();
        assert_eq!(checks.len(), 4);
        let summary = Summary { runs: vec![run] };
        let value = crate::json::parse(&summary.json()).unwrap();
        assert_eq!(
            value.get("full_volume"),
            Some(&crate::json::Json::Bool(false))
        );
        assert_eq!(
            value.get("verdict"),
            Some(&crate::json::Json::String("NO-VOLUME".into()))
        );
    }
}
