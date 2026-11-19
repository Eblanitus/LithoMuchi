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

/// ВО0-2: `S0` — это сухая поверхность `S` там, где гидрология ничего не
/// меняет.
///
/// Сравнение идёт с `dry_surface_at` (поверхность без выреза воды), а не с
/// итоговой: `with_water` добивает берег у воды. И колонка должна быть
/// «тихой»: `apply_fields` подмешивает в C/E/W силу русла, озера, поймы,
/// болота и устья, а `S0` по определению считается до гидрологии. Где влияние
/// нулевое, ответы обязаны совпасть.
#[test]
fn s0_matches_dry_surface_without_hydrology() {
    let g = Generator::new(42, Mode::Gameplay, GenVersion::V1DevW).unwrap();
    let mut checked = 0;
    for (x, z) in points().into_iter().take(256) {
        let Some(world) = g.world_sample(x, z) else {
            continue;
        };
        let h = &world.hydrology;
        let quiet = world.river == 0.0
            && h.channels.nearest.is_none()
            && h.lakes.water_hit.is_none()
            && h.water == 0.0
            && h.valley == 0.0
            && h.floodplain == 0.0
            && h.swamp == 0.0
            && h.estuary == 0.0
            && h.gallery == 0.0
            && h.r2_deferred == 0.0;
        if !quiet {
            continue;
        }
        let Some(dry) = g.dry_surface_at(x, z).expect("колонка есть") else {
            continue;
        };
        let s0 = g.s0_at(x, z).expect("S0 у v1-dev-w");
        assert!(
            (s0.surface_y() - dry).abs() < 1e-6,
            "S0 {} против сухой поверхности {dry} в ({x}, {z})",
            s0.surface_y()
        );
        checked += 1;
    }
    assert!(checked >= 8, "тихих колонок мало: {checked}");
}
