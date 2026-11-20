//! ВО0 плана водного обновления (`docs/PLAN-WATER-UPDATE.md`, Д-129).
//!
//! Шаг ВО0-1: версия `v1-dev-w` — пока копия поведения `v1-dev-r2`. Проверки:
//! колонки воды совпадают побитно на 1000 колонках обоих режимов, раздел
//! параметров `water` входит в хеш только у неё (у `v0`, `v1`, `v1-dev` и
//! `v1-dev-r2` остальные строки прежние).
//!
//! Шаг ВО0-2: `S0` — сухая поверхность без рек и озёр. Вне воды она обязана
//! совпасть с поверхностью колонки, а быстрый путь `s0_node` — с точным.

use litho_core::version::GenVersion;
use litho_core::{Field, Generator, Mode, Params};

/// Те же 1000 точек, что в CI мода и в паритете Ф4 (`r2_fields_parity`).
fn points() -> Vec<(f64, f64)> {
    (0..1000)
        .map(|i| {
            let x = (i as i64 * 7919).rem_euclid(40_000) - 20_000;
            let z = (i as i64 * 104_729).rem_euclid(40_000) - 20_000;
            (x as f64, z as f64)
        })
        .collect()
}

/// ВО1-1/ВО1-3: уровни `W` считаются профилем от `S0`, а карандашные реки
/// выключены. Поэтому мир W больше не обязан совпадать с R2 ни в колонках, ни
/// в `C` (совпадение было требованием ВО0-1 и закрылось вместе с «W — копия»).
/// Здесь проверяется то, что должно держаться при любом профиле: `S0` есть и
/// конечен, вода не ниже моря, дно не выше зеркала, и ответ не зависит от
/// пересчёта (кэши уровней и `S0` не меняют мир).
#[test]
fn w_levels_and_s0_are_sane() {
    for mode in Mode::ALL {
        let first = Generator::new(42, mode, GenVersion::V1DevW).unwrap();
        let second = Generator::new(42, mode, GenVersion::V1DevW).unwrap();
        for (x, z) in points().into_iter().take(200) {
            let s0 = first.s0_at(x, z).expect("S0 есть у W");
            assert!(s0.macro_y.is_finite(), "S0 не число в ({x}, {z})");
            let column = first.water_column(x, z);
            // `WaterColumn::default()` — «воды нет» с `level = i32::MIN`,
            // поэтому спрашиваем `has_water`, а не сравниваем с нулём.
            if column.has_water() {
                assert!(
                    column.level >= 63,
                    "уровень ниже моря в ({x}, {z}), режим {mode:?}"
                );
                // Согласованность дна и зеркала (`bed ≤ level`) — гейт ВО1-2:
                // пока вырез 6.5 не сделан, поверхность считается старым путём
                // и вода нового профиля может оказаться под землёй.
                assert!(
                    column.surface >= column.bed,
                    "поверхность ниже дна ({x}, {z})"
                );
            }
            assert_eq!(
                first.field(Field::Continents, x, 0.0, z),
                second.field(Field::Continents, x, 0.0, z),
                "C зависит от порядка расчёта ({x}, {z}), режим {mode:?}"
            );
            assert_eq!(
                column,
                second.water_column(x, z),
                "колонка не сошлась ({x}, {z})"
            );
        }
    }
}

/// ВО0-1: раздел `water` (§6.9) — только у `v1-dev-w`. Прочие строки
/// канонической записи у неё те же, что у R2, поэтому хеши старых версий
/// остаются прежними (их пины проверяет CI и `params.rs`).
#[test]
fn water_section_is_w_only() {
    let strip = |text: &str| {
        text.lines()
            .filter(|line| !line.starts_with("version=") && !line.starts_with("water."))
            .collect::<Vec<_>>()
            .join("\n")
    };
    for mode in Mode::ALL {
        let w = Params::defaults(GenVersion::V1DevW, mode);
        let r2 = Params::defaults(GenVersion::V1DevR2, mode);
        let text = w.canonical_text();
        assert!(
            text.contains("water.profile_step_blocks="),
            "у W нет раздела water ({mode:?})"
        );
        for version in [GenVersion::V0, GenVersion::V1Dev, GenVersion::V1DevR2] {
            let old = Params::defaults(version, mode);
            assert!(
                !old.canonical_text().contains("water."),
                "{version:?} получила раздел water ({mode:?})"
            );
        }
        assert_ne!(w.params_hash(), r2.params_hash());
        assert_eq!(strip(&text), strip(&r2.canonical_text()));
    }
}

/// ВО0-2: `S0` не зависит от слоя воды.
///
/// Это главный инвариант шага: уровни в W считаются **от** `S0` (§6.1), и
/// если `S0` начнёт читать уровни или вырез, получится цикл «уровень → `S0` →
/// уровень». Поэтому ответ обязан совпасть до и после прогрева слоя воды
/// (`hydrology_levels_at` строит уровни R2, `water_column` — колонки).
#[test]
fn s0_is_independent_of_the_water_layer() {
    let points: Vec<(f64, f64)> = points().into_iter().take(64).collect();
    let g = Generator::new(42, Mode::Gameplay, GenVersion::V1DevW).unwrap();
    let before: Vec<_> = points.iter().map(|&(x, z)| g.s0_at(x, z)).collect();
    for &(x, z) in &points {
        let _ = g.hydrology_levels_at(x, z);
        let _ = g.water_column(x, z);
    }
    let after: Vec<_> = points.iter().map(|&(x, z)| g.s0_at(x, z)).collect();
    assert_eq!(before, after, "S0 изменился после счёта воды");
    // Заодно: значения в пределах мира.
    let mut seen = 0;
    for sample in after.into_iter().flatten() {
        assert!(sample.surface_y().is_finite(), "S0 не число");
        assert!(
            (-64.0..=319.0).contains(&sample.surface_y()),
            "S0 {} вне мира",
            sample.surface_y()
        );
        seen += 1;
    }
    assert!(seen >= 8, "S0 не посчитался почти нигде: {seen}");
}

/// Узловой путь пропускает точечный климат, но высота зависит только от
/// морфовесов, коры и берега. Проверяем его против отдельного пути с климатом.
#[test]
fn s0_node_matches_s0_at() {
    let g = Generator::new(42, Mode::Scientific, GenVersion::V1DevW).unwrap();
    let mut checked = 0;
    for (x, z) in points().into_iter().take(64) {
        let (Some(exact), Some(fast)) = (g.s0_at(x, z), g.s0_node(x, z)) else {
            continue;
        };
        assert!(
            (exact.surface_y() - fast.surface_y()).abs() < 1e-9,
            "быстрый S0 {} против точного {} в ({x}, {z})",
            fast.surface_y(),
            exact.surface_y()
        );
        checked += 1;
    }
    assert!(checked > 0, "S0 не посчитался ни в одной точке");
}

/// `S0` — понятие водного обновления: у прежних версий его нет.
#[test]
fn s0_exists_only_in_water_update() {
    for version in [GenVersion::V0, GenVersion::V1Dev, GenVersion::V1DevR2] {
        let g = Generator::new(42, Mode::Gameplay, version).unwrap();
        assert!(g.s0_at(0.0, 0.0).is_none(), "{version:?}");
        assert!(g.s0_node(0.0, 0.0).is_none(), "{version:?}");
    }
}
