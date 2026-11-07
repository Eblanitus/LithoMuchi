//! Reproduce old five-full-samples cost vs exact cached field queries.
//! cargo run -p litho-core --release --example bench_columns -- 42 gameplay 4
use litho_core::{Field, GenVersion, Generator, Mode};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seed = args
        .first()
        .map(|s| s.parse::<i64>().expect("seed i64"))
        .unwrap_or(42);
    let mode = args
        .get(1)
        .map(|s| Mode::parse(s).expect("gameplay|scientific"))
        .unwrap_or(Mode::Gameplay);
    let side = args
        .get(2)
        .map(|s| s.parse::<usize>().expect("chunk side"))
        .unwrap_or(2);
    assert!((1..=16).contains(&side), "side must be 1..16");
    let generator = Generator::new(seed, mode, GenVersion::V0).unwrap();
    let fields = &Field::ALL[..5];
    let mut expected = Vec::new();
    let start = Instant::now();
    for cz in 0..side {
        for cx in 0..side {
            for z in 0..16 {
                for x in 0..16 {
                    let x = (cx * 16 + x) as f64 - 128.0;
                    let z = (cz * 16 + z) as f64 - 128.0;
                    let mut values = [0; 5];
                    for (i, value) in values.iter_mut().enumerate() {
                        let s = std::hint::black_box(generator.sample(x, z).unwrap());
                        *value = [
                            s.vanilla.c,
                            s.vanilla.e,
                            s.vanilla.w,
                            s.vanilla.t,
                            s.vanilla.h,
                        ][i]
                            .to_bits();
                    }
                    expected.push(values);
                }
            }
        }
    }
    let old = start.elapsed();
    for field_major in [false, true] {
        Generator::clear_column_cache();
        let start = Instant::now();
        for cz in 0..side {
            for cx in 0..side {
                for outer in 0..if field_major { 5 } else { 256 } {
                    for inner in 0..if field_major { 256 } else { 5 } {
                        let (pixel, f) = if field_major {
                            (inner, outer)
                        } else {
                            (outer, inner)
                        };
                        let x = (cx * 16 + pixel % 16) as f64 - 128.0;
                        let z = (cz * 16 + pixel / 16) as f64 - 128.0;
                        let index = (cz * side + cx) * 256 + pixel;
                        assert_eq!(
                            generator.field(fields[f], x, 64.0, z).to_bits(),
                            expected[index][f]
                        );
                    }
                }
            }
        }
        let elapsed = start.elapsed();
        let (requests, hits) = Generator::column_cache_counts();
        println!(
            "seed={seed} mode={} chunks={} layout={} uncached={old:?} cached={elapsed:?} speedup={:.2} requests={requests} hits={hits}",
            mode.as_str(),
            side * side,
            if field_major {
                "field-major"
            } else {
                "point-major"
            },
            old.as_secs_f64() / elapsed.as_secs_f64()
        );
    }
}
