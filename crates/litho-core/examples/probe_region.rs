//! Reproducible core-field scan, not a report of actual in-game biome IDs.
//! seed mode center_x center_z radius step; CSV on stdout.
use litho_core::{GenVersion, Generator, Mode};
use std::io::{self, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 6 {
        return Err("usage: probe_region seed mode center_x center_z radius step".into());
    }
    let seed: i64 = args[0].parse()?;
    let mode = Mode::parse(&args[1]).ok_or("invalid mode")?;
    let cx: i32 = args[2].parse()?;
    let cz: i32 = args[3].parse()?;
    let radius: i32 = args[4].parse()?;
    let step: usize = args[5].parse()?;
    if !(0..=2048).contains(&radius) || step == 0 || step > 4096 {
        return Err("radius must be 0..2048; step 1..4096".into());
    }
    if cx.unsigned_abs() > 29_990_000 || cz.unsigned_abs() > 29_990_000 {
        return Err("center outside world limits".into());
    }
    let g = Generator::new(seed, mode, GenVersion::V0).map_err(|e| format!("{e:?}"))?;
    let mut out = io::BufWriter::new(io::stdout().lock());
    writeln!(
        out,
        "x,z,height_km,mountain,plateau,t_bio,t_min,precip,ratio,belt,province,cell_t,cell_h,label,c,e,w,t,h"
    )?;
    for z in (cz - radius..=cz + radius).step_by(step) {
        for x in (cx - radius..=cx + radius).step_by(step) {
            let s = g
                .world_sample(f64::from(x), f64::from(z))
                .ok_or("sample failed")?;
            let c = s.climate;
            let a = s.class;
            let v = s.vanilla;
            writeln!(
                out,
                "{x},{z},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                c.surface.e_km,
                c.surface.mountain,
                c.surface.plateau,
                c.temperature.t_bio,
                c.temperature.t_min,
                c.precip.p_mm,
                c.precip.ratio,
                a.zone.belt,
                a.zone.province,
                a.cell.t,
                a.cell.h,
                a.cell.biome,
                v.c,
                v.e,
                v.w,
                v.t,
                v.h
            )?;
        }
    }
    Ok(())
}
