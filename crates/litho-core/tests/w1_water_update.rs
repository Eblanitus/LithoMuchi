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
use litho_core::{Generator, Mode, Params};

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

/// ВО0-1: `v1-dev-w` меняет только параметры, а не мир. Колонка воды — это
/// уровень, дно, поверхность и вид воды; все четыре обязаны совпасть с R2.
#[test]
fn w_copies_r2_water_columns() {
    for mode in Mode::ALL {
        let w = Generator::new(42, mode, GenVersion::V1DevW).unwrap();
        let r2 = Generator::new(42, mode, GenVersion::V1DevR2).unwrap();
        for (x, z) in points() {
            assert_eq!(
                w.water_column(x, z),
                r2.water_column(x, z),
                "режим {mode:?}, колонка ({x}, {z})"
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
    // Заодно: значения в пределах мира, а вклад микробугров не больше размаха
    // Д-057 (6 блоков в научном, 4 в геймплейном) с точностью до веса берега.
    let max_micro = g
        .params()
        .height
        .micro_amp_blocks
        .iter()
        .copied()
        .fold(0.0, f64::max);
    let mut seen = 0;
    for sample in after.into_iter().flatten() {
        assert!(sample.surface_y().is_finite(), "S0 не число");
        assert!(
            (-64.0..=319.0).contains(&sample.surface_y()),
            "S0 {} вне мира",
            sample.surface_y()
        );
        assert!(
            sample.micro_dy.abs() <= max_micro + 1e-9,
            "вклад микробугров {} больше размаха {max_micro}",
            sample.micro_dy
        );
        seen += 1;
    }
    assert!(seen >= 8, "S0 не посчитался почти нигде: {seen}");
}

/// ВО0-2: быстрый путь `s0_node` пока равен точному. Разведка ВО0-5 выберет
/// приближение (0,25 L или «Б-лайт» 0,5 L), и этот тест станет тестом
/// точности: |быстрый − точный| ≤ 2 блока у 95 % узлов и ≤ 6 у 99,9 % §6.2.
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
