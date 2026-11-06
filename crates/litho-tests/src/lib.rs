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
