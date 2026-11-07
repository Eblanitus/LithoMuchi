//! Regression tests for the column cache: compare individual f64 bits, not sums.
use litho_core::{Field, GenVersion, Generator, Mode};
use std::sync::Arc;

fn expected(generator: &Generator, x: f64, z: f64) -> [u64; 7] {
    let s = generator.sample(x, z).unwrap();
    [
        s.vanilla.c,
        s.vanilla.e,
        s.vanilla.w,
        s.vanilla.t,
        s.vanilla.h,
        s.lava,
        s.climate.surface.e_km,
    ]
    .map(|v| if v.is_finite() { v } else { 0.0 }.to_bits())
}

#[test]
fn cache_matches_uncached_across_evictions_modes_and_worlds() {
    for mode in Mode::ALL {
        for seed in [0, 42, -123456789] {
            let generator = Generator::new(seed, mode, GenVersion::V0).unwrap();
            for (x, z) in [
                (-1797.0, -630.0),
                (1287.0, 832.0),
                (-16.0, 16.0),
                (0.0, -0.0),
                (0.125, -0.25),
                (30_000_000.0, -30_000_000.0),
            ] {
                let bits = expected(&generator, x, z);
                Generator::clear_column_cache();
                for reverse in [false, true] {
                    for i in 0..7 {
                        let i = if reverse { 6 - i } else { i };
                        assert_eq!(
                            generator.field(Field::ALL[i], x, 80.0, z).to_bits(),
                            bits[i]
                        );
                    }
                }
                let (requests, hits) = Generator::column_cache_counts();
                assert_eq!((requests, hits), (14, 13));
                Generator::clear_column_cache();
                assert_eq!(
                    generator.field(Field::Continents, x, 0.0, z).to_bits(),
                    bits[0]
                );
            }
        }
    }
}

#[test]
fn generator_identity_and_threads_do_not_change_answers() {
    let a = Arc::new(Generator::new(42, Mode::Gameplay, GenVersion::V0).unwrap());
    let b = Generator::new(-42, Mode::Scientific, GenVersion::V0).unwrap();
    let bits = expected(&a, -16.0, 32.0);
    for field in Field::ALL {
        b.field(field, -16.0, 0.0, 32.0);
    }
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let a = a.clone();
            std::thread::spawn(move || {
                for i in (0..7).rev() {
                    assert_eq!(
                        a.field(Field::ALL[i], -16.0, -64.0, 32.0).to_bits(),
                        bits[i]
                    );
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    for (i, field) in Field::ALL.into_iter().enumerate() {
        assert_eq!(a.field(field, -16.0, 100.0, 32.0).to_bits(), bits[i]);
    }
}

#[test]
fn aquifer_lava_is_bounded_by_plate_heat_and_deterministic() {
    for mode in Mode::ALL {
        let generator = Generator::new(42, mode, GenVersion::V0).unwrap();
        for x in [-4096.0, -64.0, 0.0, 64.0, 4096.0] {
            let heat = generator.lava_sample(x, -x);
            for y in [-80.0, -40.0, 0.0] {
                let value = generator.aquifer_lava(x, y, -x);
                assert!(value >= heat * 0.2 && value <= heat);
                assert_eq!(value.to_bits(), generator.aquifer_lava(x, y, -x).to_bits());
            }
        }
    }
}

#[test]
fn hash_collision_evicts_instead_of_returning_another_coordinate() {
    let generator = Generator::new(42, Mode::Gameplay, GenVersion::V0).unwrap();
    let a = expected(&generator, 0.0, 0.0);
    let b = expected(&generator, 17.0, 0.0);
    // With the current 4096-slot hash, (0,0) and (17,0) share slot zero.
    Generator::clear_column_cache();
    assert_eq!(
        generator.field(Field::Continents, 0.0, 0.0, 0.0).to_bits(),
        a[0]
    );
    assert_eq!(
        generator.field(Field::Continents, 17.0, 0.0, 0.0).to_bits(),
        b[0]
    );
    assert_eq!(
        generator.field(Field::Continents, 0.0, 0.0, 0.0).to_bits(),
        a[0]
    );
    assert_eq!(Generator::column_cache_counts(), (3, 0));
}
