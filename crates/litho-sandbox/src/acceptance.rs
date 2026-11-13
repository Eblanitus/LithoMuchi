//! T151/10.1: вердикт автоматических критериев границ биомов и ориентиры 12.4.
//!
//! `stats`/`stats-all` печатают числа, но не говорят «пройдено/не пройдено» и
//! не умеют считать приёмку по частям. Полный скан 16 сидов × 2 режима при
//! ≥200×200 L — это миллионы точек на прогон, поэтому нужен инструмент,
//! который (а) берёт подмножество сидов и режимов, (б) сравнивает числа с
//! целями 10.1 и (в) честно помечает объём, если он меньше приёмочного.
//!
//! Пороги здесь — цитата целей 10.1 и ориентиров 12.4, а не новые числа
//! генератора: они не входят в `params_hash` и не меняют мир. 10.1 даёт
//! PASS/FAIL/NO-VOLUME; 12.4 по букве плана — ориентиры, а не законы, поэтому
//! печатается отдельным блоком `ORIENTATION` и не влияет на вердикт приёмки
//! (Д-072, Д-073).

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

/// Диапазон 12.4 плана: ориентир, а не критерий приёмки.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub lo: f64,
    pub hi: f64,
}

pub const TARGET_LAND_PCT: Range = Range { lo: 25.0, hi: 40.0 };
pub const TARGET_MOUNTAINS_PCT: Range = Range { lo: 5.0, hi: 20.0 };
pub const TARGET_DESERTS_PCT: Range = Range { lo: 15.0, hi: 30.0 };
pub const TARGET_FORESTS_PCT: Range = Range { lo: 30.0, hi: 50.0 };
pub const TARGET_POLAR_PCT: Range = Range { lo: 10.0, hi: 25.0 };
/// Точка появления 12.4: `spawn_good` переводится в 1.0 / 0.0, диапазон [1, 1].
pub const SPAWN_GOOD: Range = Range { lo: 1.0, hi: 1.0 };

/// Строки блока 12.4 в порядке вывода. Порядок используется и в сводке.
pub const CLIMATE124_LABELS: [&str; 6] = [
    "суша 12.4, %",
    "горы 12.4, %",
    "пустыни 12.4, %",
    "леса 12.4, %",
    "полярные 12.4, %",
    "точка появления 12.4",
];

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

/// Ориентир 12.4: значение, диапазон и факт попадания. Не критерий приёмки —
/// у этого блока нет PASS/FAIL и он не входит в `RunVerdict::verdict()`.
#[derive(Clone, Copy, Debug)]
pub struct Advisory {
    pub name: &'static str,
    /// `None` — скан пустой, метрику считать не на чем.
    pub value: Option<f64>,
    pub lo: f64,
    pub hi: f64,
}

impl Advisory {
    fn range(name: &'static str, value: Option<f64>, target: Range) -> Self {
        Self {
            name,
            value,
            lo: target.lo,
            hi: target.hi,
        }
    }
    pub fn has_data(&self) -> bool {
        self.value.is_some_and(|v| v.is_finite())
    }
    pub fn within(&self) -> bool {
        self.value
            .is_some_and(|v| v.is_finite() && v >= self.lo && v <= self.hi)
    }
    pub fn text(&self) -> String {
        let value = self
            .value
            .map_or_else(|| "нет данных".to_string(), |v| format!("{v:.3}"));
        let state = if !self.has_data() {
            "НЕТ ДАННЫХ"
        } else if self.within() {
            "в ориентире"
        } else {
            "ВНЕ ОРИЕНТИРА"
        };
        format!(
            "{:<26} {:>12}  [{:>5.2}, {:>5.2}]  {}",
            self.name, value, self.lo, self.hi, state
        )
    }
    pub fn json(&self) -> String {
        let value = self
            .value
            .map_or_else(|| "null".to_string(), |v| format!("{v:.6}"));
        format!(
            "{{\"name\": {}, \"value\": {}, \"lo\": {:.6}, \"hi\": {:.6}, \
             \"within\": {}, \"verdict\": {}}}",
            crate::json::quote(self.name),
            value,
            self.lo,
            self.hi,
            self.within(),
            crate::json::quote("ORIENTATION")
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
    /// 12.4: ориентиры, не влияющие на вердикт 10.1.
    pub advisory: Vec<Advisory>,
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
        // Пустой скан не выдаёт «вне ориентира»: значения просто не посчитаны.
        let measured = stats.samples > 0;
        let land_measured = stats.land_c > 0;
        let opt = |ok: bool, v: f64| ok.then_some(v);
        let advisory = vec![
            Advisory::range(
                CLIMATE124_LABELS[0],
                opt(measured, stats.land_c_pct()),
                TARGET_LAND_PCT,
            ),
            Advisory::range(
                CLIMATE124_LABELS[1],
                opt(land_measured, stats.mountains_c_pct()),
                TARGET_MOUNTAINS_PCT,
            ),
            Advisory::range(
                CLIMATE124_LABELS[2],
                opt(land_measured, stats.deserts_c_pct()),
                TARGET_DESERTS_PCT,
            ),
            Advisory::range(
                CLIMATE124_LABELS[3],
                opt(land_measured, stats.forests_c_pct()),
                TARGET_FORESTS_PCT,
            ),
            Advisory::range(
                CLIMATE124_LABELS[4],
                opt(land_measured, stats.polar_c_pct()),
                TARGET_POLAR_PCT,
            ),
            Advisory::range(
                CLIMATE124_LABELS[5],
                opt(measured, if stats.spawn_good { 1.0 } else { 0.0 }),
                SPAWN_GOOD,
            ),
        ];
        Self {
            seed: stats.seed,
            mode: stats.mode.as_str(),
            checks,
            acceptance_volume: volume,
            advisory,
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
    /// Вердикт по прогону: PASS только если объём приёмочный и все проверки
    /// 10.1 прошли. Ориентиры 12.4 сюда не входят (Д-072, Д-073).
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
        out.push_str("  12.4 — ORIENTATION, ориентиры не влияют на приёмку 10.1:\n");
        for advisory in &self.advisory {
            out.push_str("    ");
            out.push_str(&advisory.text());
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
        let advisory = self
            .advisory
            .iter()
            .map(Advisory::json)
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{{\"seed\": {}, \"mode\": {}, \"acceptance_volume\": {}, \
             \"verdict\": {}, \"checks\": [{}], \
             \"climate124\": {{\"verdict\": {}, \"checks\": [{}]}}}}",
            self.seed,
            crate::json::quote(self.mode),
            self.acceptance_volume,
            crate::json::quote(self.verdict().as_str()),
            checks,
            crate::json::quote("ORIENTATION"),
            advisory
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
    /// По каждой строке 12.4: (имя, в ориентире, вне ориентира, нет данных).
    /// Считается по всем прогонам сводки, как столбец «В цели N/16» в T151.
    pub fn advisory_counts(&self) -> Vec<(&'static str, usize, usize, usize)> {
        CLIMATE124_LABELS
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let (mut within, mut outside, mut no_data) = (0, 0, 0);
                for run in &self.runs {
                    match run.advisory.get(i) {
                        Some(a) if a.has_data() => {
                            if a.within() {
                                within += 1;
                            } else {
                                outside += 1;
                            }
                        }
                        _ => no_data += 1,
                    }
                }
                (*name, within, outside, no_data)
            })
            .collect()
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
        out.push_str("ИТОГ 12.4 (ориентиры, не приёмка): вердикт ORIENTATION\n");
        for (name, within, outside, no_data) in self.advisory_counts() {
            out.push_str(&format!(
                "  {name:<26} в ориентире {within}, вне {outside}, нет данных {no_data}\n"
            ));
        }
        out
    }
    pub fn json(&self) -> String {
        let rows = self
            .advisory_counts()
            .iter()
            .map(|(name, within, outside, no_data)| {
                format!(
                    "{{\"name\": {}, \"within\": {}, \"outside\": {}, \"no_data\": {}}}",
                    crate::json::quote(name),
                    within,
                    outside,
                    no_data
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{{\"runs\": [{}], \"passed\": {}, \"failed\": {}, \"pending\": {}, \
             \"full_volume\": {}, \"verdict\": {}, \
             \"climate124\": {{\"verdict\": \"ORIENTATION\", \"rows\": [{}]}}}}",
            self.runs
                .iter()
                .map(RunVerdict::json)
                .collect::<Vec<_>>()
                .join(", "),
            self.passed(),
            self.failed(),
            self.pending(),
            self.is_full_volume(),
            crate::json::quote(self.verdict().as_str()),
            rows
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

    /// Приёмочный объём 10.1 плюс заданные доли 12.4. `parts` — горы,
    /// пустыни, леса, полярные в процентах от `land_c`.
    fn climate124(seed: i64, land_pct: f64, parts: [f64; 4], spawn_good: bool) -> HeightStats {
        let mut s = full(seed, 0.5);
        s.samples = 10_000;
        s.land_c = (land_pct * 100.0) as usize;
        let land = s.land_c as f64;
        s.mountains_c = (parts[0] / 100.0 * land) as usize;
        s.deserts_c = (parts[1] / 100.0 * land) as usize;
        s.forests_c = (parts[2] / 100.0 * land) as usize;
        s.polar_c = (parts[3] / 100.0 * land) as usize;
        s.spawn_good = spawn_good;
        s
    }

    #[test]
    fn acceptance_targets_match_section_10_1() {
        assert_eq!(TARGET_JUMP_CLIMATE_PCT, 2.0);
        assert_eq!(TARGET_WIND_DEG, 5.0);
        assert_eq!(TARGET_STRAIGHT_PCT, 1.0);
        assert_eq!(TARGET_THIN_PCT, 0.0);
    }

    #[test]
    fn advisory_targets_match_section_12_4() {
        assert_eq!(TARGET_LAND_PCT, Range { lo: 25.0, hi: 40.0 });
        assert_eq!(TARGET_MOUNTAINS_PCT, Range { lo: 5.0, hi: 20.0 });
        assert_eq!(TARGET_DESERTS_PCT, Range { lo: 15.0, hi: 30.0 });
        assert_eq!(TARGET_FORESTS_PCT, Range { lo: 30.0, hi: 50.0 });
        assert_eq!(TARGET_POLAR_PCT, Range { lo: 10.0, hi: 25.0 });
        assert_eq!(SPAWN_GOOD, Range { lo: 1.0, hi: 1.0 });
        assert_eq!(CLIMATE124_LABELS.len(), 6);
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

    #[test]
    fn climate124_within_ranges_is_reported_but_not_a_verdict() {
        let s = climate124(42, 32.0, [10.0, 20.0, 40.0, 15.0], true);
        let run = RunVerdict::from_stats(&s);
        assert_eq!(run.advisory.len(), 6);
        assert!(run.advisory.iter().all(Advisory::within));
        assert_eq!(run.verdict(), Verdict::Pass);
        assert!(run.text().contains("ORIENTATION"));
        assert!(!run.text().contains("ВНЕ ОРИЕНТИРА"));
    }

    #[test]
    fn climate124_outside_ranges_does_not_fail_10_1() {
        // Как в climate124-1: суша и полярные вне ориентира, точка в цели.
        let s = climate124(42, 51.3, [4.4, 14.7, 36.1, 7.9], true);
        let run = RunVerdict::from_stats(&s);
        assert_eq!(run.verdict(), Verdict::Pass, "12.4 не критерий приёмки");
        assert_eq!(run.failed(), 0);
        assert!(run.advisory[0].has_data() && !run.advisory[0].within());
        assert!(run.advisory[1].has_data() && !run.advisory[1].within());
        assert!(run.advisory[4].has_data() && !run.advisory[4].within());
        assert!(run.advisory[5].within());
        assert!(run.text().contains("ВНЕ ОРИЕНТИРА"));
    }

    #[test]
    fn bad_spawn_is_only_an_advisory() {
        let s = climate124(42, 32.0, [10.0, 20.0, 40.0, 15.0], false);
        let run = RunVerdict::from_stats(&s);
        assert_eq!(run.verdict(), Verdict::Pass);
        assert!(!run.advisory[5].within());
        assert!(run.advisory[5].has_data());
    }

    #[test]
    fn empty_scan_has_no_12_4_data() {
        let run = RunVerdict::from_stats(&stats(42, BoundaryMetrics::empty(64, 32.0, 250.0)));
        assert_eq!(run.advisory.len(), 6);
        assert!(run.advisory.iter().all(|a| !a.has_data()));
        assert!(run.advisory.iter().all(|a| !a.within()));
        assert!(run.text().contains("НЕТ ДАННЫХ"));
    }

    #[test]
    fn nan_12_4_value_is_no_data_not_within() {
        let mut s = climate124(42, 32.0, [10.0, 20.0, 40.0, 15.0], true);
        s.spawn_shift_l = (f64::NAN, 0.0);
        let mut a = Advisory::range("тест", Some(f64::NAN), TARGET_LAND_PCT);
        assert!(!a.has_data() && !a.within());
        a.value = None;
        assert!(!a.has_data() && !a.within());
        // Носитель с сеткой, но без точек суши: доли на суше не посчитаны.
        s.land_c = 0;
        let run = RunVerdict::from_stats(&s);
        assert!(run.advisory[0].has_data());
        assert!(!run.advisory[1].has_data());
    }

    #[test]
    fn summary_counts_12_4_rows_and_json_is_parseable() {
        let mut summary = Summary::default();
        summary.push(RunVerdict::from_stats(&climate124(
            42,
            32.0,
            [10.0, 20.0, 40.0, 15.0],
            true,
        )));
        summary.push(RunVerdict::from_stats(&climate124(
            7,
            51.3,
            [4.4, 14.7, 36.1, 7.9],
            true,
        )));
        let rows = summary.advisory_counts();
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[0], (CLIMATE124_LABELS[0], 1, 1, 0));
        assert_eq!(rows[5], (CLIMATE124_LABELS[5], 2, 0, 0));
        assert!(summary.text().contains("вердикт ORIENTATION"));
        let value = crate::json::parse(&summary.json()).unwrap();
        let block = value.get("climate124").unwrap();
        assert_eq!(
            block.get("verdict").and_then(|v| v.as_str()),
            Some("ORIENTATION")
        );
        assert_eq!(
            block.get("rows").and_then(|v| v.as_array()).unwrap().len(),
            6
        );
        // Сам вердикт сводки остаётся 10.1 и не зависит от ориентиров.
        assert_eq!(
            value.get("verdict"),
            Some(&crate::json::Json::String("NO-VOLUME".into()))
        );
    }

    #[test]
    fn run_json_12_4_block_is_advisory() {
        let run = RunVerdict::from_stats(&climate124(42, 32.0, [10.0, 20.0, 40.0, 15.0], true));
        let value = crate::json::parse(&run.json()).unwrap();
        let block = value.get("climate124").unwrap();
        assert_eq!(
            block.get("verdict").and_then(|v| v.as_str()),
            Some("ORIENTATION")
        );
        let checks = block.get("checks").and_then(|v| v.as_array()).unwrap();
        assert_eq!(checks.len(), 6);
        assert_eq!(
            checks[0].get("name").and_then(|v| v.as_str()),
            Some(CLIMATE124_LABELS[0])
        );
        assert_eq!(
            value.get("verdict"),
            Some(&crate::json::Json::String("PASS".into()))
        );
    }
}
