//! Д-128: два генератора с одним сидом и разными параметрами не должны читать
//! тайлы друг друга. Паритет Ф4 давал 20 расхождений из 5000 у `v1-dev-r2`
//! против `v1-dev` именно из-за потоковой памяти «последний coarse-тайл»: её
//! ключ не включал хеш параметров.
//!
//! Проверка идёт в том же порядке, что и в игре: сначала считает `v1-dev`,
//! потом `v1-dev-r2`. Поля `C/E/W/T/H` у R2 обязаны совпасть с `v1-dev`
//! побитово (§4 плана: R2 меняет только слой воды).

use litho_core::field::Field;
use litho_core::version::GenVersion;
use litho_core::{Generator, Mode};

const FIELDS: [Field; 5] = [
    Field::Continents,
    Field::Erosion,
    Field::Ridges,
    Field::Temperature,
    Field::Vegetation,
];

/// Те же 1000 точек, что в CI мода (`LithoCi`, шаг 7919/104729 в окне ±20 000).
fn points() -> Vec<(f64, f64)> {
    (0..1000)
        .map(|i| {
            let x = (i as i64 * 7919).rem_euclid(40_000) - 20_000;
            let z = (i as i64 * 104_729).rem_euclid(40_000) - 20_000;
            (x as f64, z as f64)
        })
        .collect()
}

#[test]
fn r2_fields_match_v1_dev_when_both_live_in_one_thread() {
    let v1 = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).expect("v1-dev");
    let r2 = Generator::new(42, Mode::Gameplay, GenVersion::V1DevR2).expect("v1-dev-r2");
    for (x, z) in points() {
        for field in FIELDS {
            let a = v1.field(field, x, 64.0, z);
            let b = r2.field(field, x, 64.0, z);
            if (a - b).abs() >= 1e-9 {
                // Диагностика: что ещё отличается в этой точке.
                let dump = |g: &Generator| {
                    g.sample(x, z).map_or_else(
                        || "нет сэмпла".to_string(),
                        |s| {
                            format!(
                                "c={} e={} w={} t={} h={} ocean={} river={:.3} surface={:.2} pv={:.3} kind={:?}",
                                s.world.vanilla.c,
                                s.world.vanilla.e,
                                s.world.vanilla.w,
                                s.world.vanilla.t,
                                s.world.vanilla.h,
                                s.world.is_ocean,
                                s.world.river,
                                s.world.height.surface_y,
                                s.world.pv,
                                s.world.hydrology.kind,
                            )
                        },
                    )
                };
                panic!(
                    "{field:?} в ({x}, {z}): v1-dev {a}, v1-dev-r2 {b}\n v1: {}\n r2: {}",
                    dump(&v1),
                    dump(&r2)
                );
            }
        }
    }
}
