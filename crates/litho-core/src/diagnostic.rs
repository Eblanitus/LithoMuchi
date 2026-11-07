//! Read-only reproducible diagnostics; explicit seed/mode, not active-world detection.
use crate::{Field, GenVersion, Generator, Mode};
use std::fmt::Write;

pub fn point(seed: i64, mode: Mode, x: f64, z: f64) -> Result<String, String> {
    if !x.is_finite() || !z.is_finite() || x.abs() > 30_000_000.0 || z.abs() > 30_000_000.0 {
        return Err("coordinates must be finite and inside +/-30000000".into());
    }
    let generator = Generator::new(seed, mode, GenVersion::V0).map_err(|e| format!("{e:?}"))?;
    let s = generator.sample(x, z).ok_or("sample failed")?;
    let mut out = format!(
        "Litospherity v0 seed={seed} mode={} at={x},{z} params={:016x}\nExplicit inputs; not a report of the active world's biome.\n",
        mode.as_str(),
        generator.params().params_hash()
    );
    for field in &Field::ALL[..5] {
        let v = generator.field(*field, x, 0.0, z);
        writeln!(
            out,
            "{}={:.9} bits={:016x} f32={:08x}",
            field.as_str(),
            v,
            v.to_bits(),
            (v as f32).to_bits()
        )
        .unwrap();
    }
    writeln!(
        out,
        "height_km={} temperature_c={} precip_mm={} zone={:?} plate_heat={}",
        s.climate.surface.e_km, s.climate.temperature.t_bio, s.climate.precip.p_mm, s.zone, s.lava
    )
    .unwrap();
    for field in [Field::Continents, Field::Erosion, Field::Ridges] {
        let mut maximum = 0.0_f64;
        for dx in [-1.0, 0.0, 1.0] {
            for dz in [-1.0, 0.0, 1.0] {
                let v = generator.field(field, x + dx, 0.0, z + dz);
                let vx = generator.field(field, x + dx + 1.0, 0.0, z + dz);
                let vz = generator.field(field, x + dx, 0.0, z + dz + 1.0);
                maximum = maximum.max((v - vx).abs()).max((v - vz).abs());
            }
        }
        writeln!(
            out,
            "local_delta_{}={} threshold=0.02",
            field.as_str(),
            maximum
        )
        .unwrap();
    }
    for y in [-80.0, -40.0] {
        writeln!(
            out,
            "aquifer_lava(y={y})={}",
            generator.aquifer_lava(x, y, z)
        )
        .unwrap();
    }
    Ok(out)
}
