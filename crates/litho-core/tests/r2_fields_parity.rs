//! Раздел 4 плана воды (PLAN-WATER-FABRIC) и паритет Ф4: `v1-dev-r2` меняет
//! **только слой воды**. Поля C/E/W/T/H и сухая поверхность `S` у R2 обязаны
//! совпасть с `v1-dev` побитово, иначе мир строит сушу по одним полям, а ядро
//! ставит воду по другим, и вода уходит в пустоту.
//!
//! Два независимых механизма расхождения, каждый со своим тестом:
//!
//! 1. `r2_fields_match_v1_dev_when_both_live_in_one_thread` — потоковая память
//!    «последний coarse-тайл». Ключ не включал хеш параметров, поэтому два
//!    генератора с одним сидом (v1-dev и v1-dev-r2) читали тайлы друг друга.
//! 2. `r2_alone_still_matches_v1_dev` — сама гидрология. `apply_fields`
//!    переводит `inland_sea` и `floodplain` в C/E/W, а в R2 предел
//!    `hydrology.r1_max_level_km` снят: озеро выше предела перестаёт быть
//!    `DeferredWater` и меняет сушу. Этот тест гоняет **каждый генератор
//!    отдельно**, поэтому кэши тайлов тут ни при чём, — так причина отделена
//!    от потока и не может быть списана на него.

use litho_core::field::Field;
use litho_core::version::GenVersion;
use litho_core::{Generator, Mode};

/// Водные биомы ванили: настоящая вода, а не отложенная котловина.
/// `DeferredWater` в `v1-dev` водного биома не даёт (Д-037, Д-070).
const WATER_BIOMES: [&str; 12] = [
    "ocean",
    "deep_ocean",
    "warm_ocean",
    "lukewarm_ocean",
    "cold_ocean",
    "deep_cold_ocean",
    "frozen_ocean",
    "deep_frozen_ocean",
    "river",
    "frozen_river",
    "swamp",
    "mangrove_swamp",
];

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

fn dump(g: &Generator, x: f64, z: f64) -> String {
    g.sample(x, z).map_or_else(
        || "нет сэмпла".to_string(),
        |s| {
            format!(
                "c={} e={} w={} ocean={} surface={:.2} S={:?} pv={:.3} e_km={:.5} kind={:?} water={:.3}",
                s.world.vanilla.c,
                s.world.vanilla.e,
                s.world.vanilla.w,
                s.world.is_ocean,
                s.world.height.surface_y,
                g.dry_surface_at(x, z).ok().flatten(),
                s.world.pv,
                s.world.climate.surface.e_km,
                s.world.hydrology.kind,
                s.world.hydrology.water,
            )
        },
    )
}

fn fields_of(g: &Generator, points: &[(f64, f64)]) -> Vec<Vec<f64>> {
    points
        .iter()
        .map(|(x, z)| FIELDS.iter().map(|f| g.field(*f, 64.0, *x, *z)).collect())
        .collect()
}

/// Причина 1: тайлы coarse-решётки не должны переезжать между генераторами.
/// Порядок как в игре: сначала считает `v1-dev`, потом `v1-dev-r2` в том же
/// потоке.
#[test]
fn r2_fields_match_v1_dev_when_both_live_in_one_thread() {
    let points = points();
    let v1 = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).expect("v1-dev");
    let r2 = Generator::new(42, Mode::Gameplay, GenVersion::V1DevR2).expect("v1-dev-r2");
    for (x, z) in &points {
        for field in FIELDS {
            let a = v1.field(field, 64.0, *x, *z);
            let b = r2.field(field, 64.0, *x, *z);
            assert!(
                (a - b).abs() < 1e-9,
                "{field:?} в ({x}, {z}): v1-dev {a}, v1-dev-r2 {b}\n v1: {}\n r2: {}",
                dump(&v1, *x, *z),
                dump(&r2, *x, *z)
            );
        }
    }
}

/// Причина 2: гидрология. Каждый генератор считает **свой** список точек
/// заново и без соседа в потоке, поэтому потоковая память тайлов здесь не
/// может ни помочь, ни помешать. Если этот тест красный, дело в полях, а не
/// в кэше.
#[test]
fn r2_alone_still_matches_v1_dev() {
    let points = points();
    let v1 = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).expect("v1-dev");
    let alone = fields_of(&v1, &points);
    let r2 = Generator::new(42, Mode::Gameplay, GenVersion::V1DevR2).expect("v1-dev-r2");
    for (k, (x, z)) in points.iter().enumerate() {
        for (n, field) in FIELDS.iter().enumerate() {
            let a = alone[k][n];
            let b = r2.field(*field, 64.0, *x, *z);
            assert!(
                (a - b).abs() < 1e-9,
                "{field:?} в ({x}, {z}): v1-dev {a} (отдельно), v1-dev-r2 {b}\n v1: {}\n r2: {}",
                dump(&v1, *x, *z),
                dump(&r2, *x, *z)
            );
        }
    }
}

/// Строка 10.1 «вода не уходит за берега» на уровне биома: `v1-dev-r2` не
/// ставит водный биом там, где `v1-dev` держит сушу. Это тот же инвариант,
/// что паритет полей, но проверенный там, где он виден игроку.
///
/// Идёт по тем же 1000 точкам и в том же потоке, что и первый тест, — так
/// ловятся обе причины сразу: и потоковая память, и гидрология (Д-127).
#[test]
fn r2_never_puts_water_biome_where_v1_dev_has_land() {
    let points = points();
    let v1 = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).expect("v1-dev");
    let r2 = Generator::new(42, Mode::Gameplay, GenVersion::V1DevR2).expect("v1-dev-r2");
    let mut checked = 0;
    for (x, z) in &points {
        let (Some(a), Some(b)) = (v1.sample(*x, *z), r2.sample(*x, *z)) else {
            continue;
        };
        let land_before = a.world.vanilla.c >= v1.params().climate.ocean_c_threshold
            && !WATER_BIOMES.contains(&a.world.class.cell.biome.as_str());
        let water_after = WATER_BIOMES.contains(&b.world.class.cell.biome.as_str());
        if land_before && water_after {
            panic!(
                "({x}, {z}): v1-dev суша «{}», v1-dev-r2 вода «{}»\n v1: {}\n r2: {}",
                a.world.class.cell.biome,
                b.world.class.cell.biome,
                dump(&v1, *x, *z),
                dump(&r2, *x, *z)
            );
        }
        if !water_after {
            checked += 1;
        }
    }
    assert!(
        checked > 800,
        "проверено всего {checked} сухих колонок: сетка точек не та"
    );
}
