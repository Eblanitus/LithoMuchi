//! Инварианты 14.2 и побитовые эталоны 14.3.

#[cfg(test)]
mod golden {
    use std::collections::HashMap;

    use litho_core::hash::{fnv1a64, hash3, splitmix64, unit};
    use litho_core::math::{clamp, dexp, dln, dpow, lerp, piecewise, smoothstep};

    /// Сверяет биты результатов с `golden/v0/primitives.txt`.
    /// Запускается в CI на Linux, Windows msvc и Windows gnu: расхождение
    /// хотя бы на одной платформе означает нарушение правил 6.3.
    #[test]
    fn primitives_match_bit_for_bit() {
        let mut expected: HashMap<String, u64> = HashMap::new();
        for line in include_str!("../../../golden/v0/primitives.txt").lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let name = parts.next().expect("имя эталона");
            let hex = parts.next().expect("значение эталона");
            expected.insert(name.to_string(), u64::from_str_radix(hex, 16).expect("hex"));
        }

        let h = hash3(42, 1, -2, 3);
        let actual: Vec<(&str, u64)> = vec![
            ("splitmix64_0", splitmix64(0)),
            ("splitmix64_1", splitmix64(1)),
            ("splitmix64_max", splitmix64(u64::MAX)),
            ("splitmix64_deadbeef", splitmix64(0xDEAD_BEEF)),
            ("fnv1a64_plates.seed", fnv1a64("plates.seed")),
            ("fnv1a64_poles.seed", fnv1a64("poles.seed")),
            ("fnv1a64_empty", fnv1a64("")),
            ("hash3_42_1_-2_3", h),
            ("hash3_0_0_0_0", hash3(0, 0, 0, 0)),
            (
                "hash3_deadbeef_-123456789_987654321_-1",
                hash3(0xDEAD_BEEF, -123_456_789, 987_654_321, -1),
            ),
            ("unit_0", unit(0).to_bits()),
            ("unit_max", unit(u64::MAX).to_bits()),
            ("unit_hash3_42_1_-2_3", unit(h).to_bits()),
            ("smoothstep_0_1_0.3", smoothstep(0.0, 1.0, 0.3).to_bits()),
            ("smoothstep_1_0_0.3", smoothstep(1.0, 0.0, 0.3).to_bits()),
            ("smoothstep_-1_2_0", smoothstep(-1.0, 2.0, 0.0).to_bits()),
            ("lerp_1_3_0.25", lerp(1.0, 3.0, 0.25).to_bits()),
            ("clamp_-2_0_1", clamp(-2.0, 0.0, 1.0).to_bits()),
            (
                "piecewise_1.5",
                piecewise(&[0.0, 1.0, 2.0], &[10.0, 20.0, 40.0], 1.5).to_bits(),
            ),
            ("dexp_0", dexp(0.0).to_bits()),
            ("dexp_1", dexp(1.0).to_bits()),
            ("dexp_-3.5", dexp(-3.5).to_bits()),
            ("dexp_12", dexp(12.0).to_bits()),
            ("dln_1", dln(1.0).to_bits()),
            ("dln_2", dln(2.0).to_bits()),
            ("dln_0.5", dln(0.5).to_bits()),
            ("dln_10", dln(10.0).to_bits()),
            ("dpow_0.5_1.6", dpow(0.5, 1.6).to_bits()),
            ("dpow_2_0.5", dpow(2.0, 0.5).to_bits()),
        ];

        assert_eq!(
            expected.len(),
            actual.len(),
            "число эталонов в файле не совпадает с числом проверок"
        );
        for (name, bits) in actual {
            let golden = expected
                .get(name)
                .unwrap_or_else(|| panic!("нет эталона '{name}'"));
            assert_eq!(*golden, bits, "эталон '{name}'");
        }
    }
}

#[cfg(test)]
mod invariants {
    use litho_core::hash::{splitmix64, unit};
    use litho_core::noise::noise2;
    use litho_core::{GenVersion, Generator, Mode};

    fn points(count: usize) -> Vec<(f64, f64)> {
        (0..count)
            .map(|k| {
                (
                    unit(splitmix64(k as u64)) * 20_000.0 - 10_000.0,
                    unit(splitmix64(k as u64 + 777_777)) * 20_000.0 - 10_000.0,
                )
            })
            .collect()
    }

    /// 14.2: результат не зависит от порядка вычислений.
    #[test]
    fn plate_ids_do_not_depend_on_order() {
        let generator = Generator::new(42, Mode::Gameplay, GenVersion::V0).unwrap();
        let pts = points(2000);
        let ids: Vec<u64> = pts
            .iter()
            .map(|(x, z)| generator.plate_sample(*x, *z).unwrap().plate_id())
            .collect();
        let reversed: Vec<u64> = pts
            .iter()
            .rev()
            .map(|(x, z)| generator.plate_sample(*x, *z).unwrap().plate_id())
            .rev()
            .collect();
        assert_eq!(ids, reversed);
    }

    /// 14.2: одна планета в двух масштабах (8.2).
    #[test]
    fn one_planet_two_scales() {
        let scientific = Generator::new(-123_456_789, Mode::Scientific, GenVersion::V0).unwrap();
        let gameplay = Generator::new(-123_456_789, Mode::Gameplay, GenVersion::V0).unwrap();
        for (x, z) in points(1000) {
            assert_eq!(
                gameplay.macro_point(x, z),
                scientific.macro_point(4.0 * x, 4.0 * z)
            );
            assert_eq!(
                gameplay.plate_sample(x, z).unwrap().plate_id(),
                scientific
                    .plate_sample(4.0 * x, 4.0 * z)
                    .unwrap()
                    .plate_id()
            );
        }
    }

    /// 14.2: никаких NaN/бесконечности, в том числе на краю мира.
    #[test]
    fn all_values_are_finite() {
        let generator = Generator::new(7, Mode::Scientific, GenVersion::V0).unwrap();
        for (x, z) in points(500) {
            let j = generator.jacobian(x, z);
            assert!(j.is_finite(), "якобиан = {j}");
            let sample = generator.plate_sample(x, z).unwrap();
            let d = sample.d_ab_l;
            assert!(d.is_finite(), "d_AB = {d}");
            assert!(sample.w_b.is_finite() && sample.w_c.is_finite());
            assert!(sample.a.velocity_at(sample.q).len().is_finite());
        }
        for x in [-30_000_000.0_f64, 30_000_000.0] {
            for z in [-30_000_000.0_f64, 30_000_000.0] {
                assert!(generator.jacobian(x, z).is_finite());
                assert!(generator.plate_sample(x, z).is_some());
                let p = generator.macro_point(x, z);
                assert!(p.len().is_finite());
            }
        }
    }

    /// 14.2: макро-рельеф одной планеты побитово равен в двух режимах.
    #[test]
    fn crust_matches_across_scales() {
        let scientific = Generator::new(42, Mode::Scientific, GenVersion::V0).unwrap();
        let gameplay = Generator::new(42, Mode::Gameplay, GenVersion::V0).unwrap();
        for (x, z) in points(200) {
            let a = gameplay.crust_sample(x, z).unwrap();
            let b = scientific.crust_sample(4.0 * x, 4.0 * z).unwrap();
            assert_eq!(a.e_km.to_bits(), b.e_km.to_bits());
            assert_eq!(a.kappa_l.to_bits(), b.kappa_l.to_bits());
            assert_eq!(a.mountain.to_bits(), b.mountain.to_bits());
        }
    }

    #[test]
    fn noise_is_bounded_on_a_large_grid() {
        for k in 0..50_000 {
            let x = k as f64 * 0.031;
            let z = k as f64 * -0.017;
            let n = noise2(99, x, z);
            assert!((-1.0..=1.0).contains(&n), "noise = {n}");
        }
    }
}

#[cfg(test)]
mod fast_path {
    use litho_core::hash::{splitmix64, unit};
    use litho_core::{Field, GenVersion, Generator, Mode};

    /// Маленькая область: грубая решётка считается тайлами целиком, поэтому
    /// для проверки соответствия достаточно нескольких соседних тайлов (14.2).
    fn points(count: usize) -> Vec<(f64, f64)> {
        (0..count)
            .map(|k| {
                (
                    unit(splitmix64(k as u64 + 11)) * 800.0 - 400.0,
                    unit(splitmix64(k as u64 + 900_001)) * 800.0 - 400.0,
                )
            })
            .collect()
    }

    fn generator() -> Generator {
        Generator::new(42, Mode::Gameplay, GenVersion::V0).unwrap()
    }

    /// 14.2: быстрый путь близок к эталону по C/E/W и классу зоны.
    #[test]
    fn fast_path_matches_reference() {
        let generator = generator();
        let pts = points(300);
        let mut same_zone = 0_usize;
        for (x, z) in &pts {
            let reference = generator.sample_reference(*x, *z).unwrap();
            let fast = generator.sample(*x, *z).unwrap();
            for (name, a, b) in [
                ("C", reference.vanilla.c, fast.vanilla.c),
                ("E", reference.vanilla.e, fast.vanilla.e),
                ("W", reference.vanilla.w, fast.vanilla.w),
            ] {
                assert!(
                    (a - b).abs() <= 0.03,
                    "{name}: эталон {a}, быстрый {b} в ({x}, {z})"
                );
            }
            if reference.zone == fast.zone {
                same_zone += 1;
            }
        }
        let share = same_zone as f64 / pts.len() as f64;
        assert!(
            share >= 0.97,
            "совпадение зон {share:.3}, нужно не меньше 0.97"
        );
    }

    /// 14.2: кэш не меняет ответ, результаты не зависят от порядка.
    #[test]
    fn cache_does_not_change_the_answer() {
        let generator = generator();
        let pts = points(120);
        let forward: Vec<u64> = pts
            .iter()
            .map(|(x, z)| generator.field(Field::Continents, *x, 64.0, *z).to_bits())
            .collect();
        let backward: Vec<u64> = pts
            .iter()
            .rev()
            .map(|(x, z)| generator.field(Field::Continents, *x, 64.0, *z).to_bits())
            .rev()
            .collect();
        assert_eq!(forward, backward);
        let cold: Vec<u64> = pts
            .iter()
            .map(|(x, z)| {
                let fresh = Generator::new(42, Mode::Gameplay, GenVersion::V0).unwrap();
                fresh.field(Field::Continents, *x, 64.0, *z).to_bits()
            })
            .collect();
        assert_eq!(forward, cold);
    }

    /// 14.2: между соседними блоками нет скачков C/E/W больше 0,02.
    #[test]
    fn adjacent_blocks_are_continuous() {
        let generator = generator();
        for (x, z) in points(100) {
            for field in [Field::Continents, Field::Erosion, Field::Ridges] {
                let here = generator.field(field, x, 64.0, z);
                let east = generator.field(field, x + 1.0, 64.0, z);
                let south = generator.field(field, x, 64.0, z + 1.0);
                assert!((here - east).abs() <= 0.02, "{field:?} на восток");
                assert!((here - south).abs() <= 0.02, "{field:?} на юг");
            }
        }
    }

    /// 14.2: ни NaN, ни бесконечности, в том числе на краю мира.
    #[test]
    fn fields_are_finite_at_the_world_edge() {
        let generator = generator();
        for x in [-30_000_000.0_f64, -1.0, 0.0, 1.0, 30_000_000.0] {
            for z in [-30_000_000.0_f64, -1.0, 0.0, 1.0, 30_000_000.0] {
                for field in Field::ALL {
                    let value = generator.field(field, x, -54.0, z);
                    assert!(value.is_finite(), "{field:?} в ({x}, {z}) = {value}");
                    assert!(generator.field_f32(field, x, -54.0, z).is_finite());
                }
            }
        }
    }
}
