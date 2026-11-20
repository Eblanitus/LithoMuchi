//! Paired core benchmark. Both arms visit the same 256 integer columns in
//! each chunk exactly once. Only the point-feature switch differs. Network
//! warmup must not touch a point site or populate the measured column cache.
use litho_core::generator::WaterColumn;
use litho_core::{GenVersion, Generator, Mode};
use std::time::{Duration, Instant};

pub fn run(seed: i64, mode: Mode, center: (f64, f64), side: usize) -> Result<(), String> {
    if !(1..=32).contains(&side) {
        return Err("--chunks-side должен быть 1…32".into());
    }
    let mut params = crate::tune::params(mode, GenVersion::V1DevW)?;
    params.water.point_lakes = false;
    let off = Generator::with_params(seed, params.clone()).map_err(|e| e.to_string())?;
    params.water.point_lakes = true;
    let on = Generator::with_params(seed, params).map_err(|e| e.to_string())?;
    let gens = [off, on];
    let mut elapsed = [Duration::ZERO; 2];
    let mut wet = [0u64; 2];
    let mut point = 0u64;
    let base = |v: f64| ((v / 16.).floor() as i64 - side as i64 / 2) * 16;
    let (x0, z0) = (base(center.0), base(center.1));
    for g in &gens {
        for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            g.hydrology_levels_at(
                (x0 + 16 * side as i64 * dx) as f64 + 0.5,
                (z0 + 16 * side as i64 * dz) as f64 + 0.5,
            )
            .map_err(|e| e.to_string())?;
        }
    }
    for k in 0..side * side {
        for arm in [k % 2, 1 - k % 2] {
            let start = Instant::now();
            for dz in 0..16 {
                for dx in 0..16 {
                    let x = x0 + (16 * (k % side) + dx) as i64;
                    let z = z0 + (16 * (k / side) + dz) as i64;
                    let c = gens[arm].water_column(x as f64, z as f64);
                    if c.surface == WaterColumn::NONE {
                        return Err(format!("нет колонки ({x},{z})"));
                    }
                    wet[arm] += u64::from(c.has_water());
                    point += u64::from(c.has_water() && c.body.is_point_lake());
                }
            }
            elapsed[arm] += start.elapsed();
        }
    }
    let [off, on] = elapsed.map(|d| d.as_secs_f64() * 1000. / (side * side) as f64);
    println!(
        "point-lake-cost: seed={seed} mode={} center={},{} chunks={} off_ms/chunk={off:.2} on_ms/chunk={on:.2} delta_pct={:.2} wet_off={} wet_on={} point_columns={point}",
        mode.as_str(),
        center.0,
        center.1,
        side * side,
        100. * (on / off - 1.),
        wet[0],
        wet[1]
    );
    Ok(())
}
