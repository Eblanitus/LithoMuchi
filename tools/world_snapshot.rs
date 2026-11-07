// Shared snapshot writer: standalone rustc utility and integration test module.
use litho_core::{Field, GenVersion, Generator, Mode};
use std::fmt::Write;

pub fn snapshot() -> String {
    let mut out = String::from(
        "# World v0 algorithm 0.3.0: seed mode x z C E W T H e P T_bio shift_x shift_z zone; f64 bits hex\n",
    );
    for seed in [0, 42, -123456789] {
        for mode in Mode::ALL {
            let generator = Generator::new(seed, mode, GenVersion::V0).unwrap();
            let l = mode.l_blocks();
            for k in 0..1056 {
                let (x, z) = if k < 1024 {
                    (
                        ((k % 32) as f64 - 16.0) * 3.7 * l,
                        ((k / 32) as f64 - 16.0) * 3.7 * l,
                    )
                } else {
                    let h = litho_core::hash::hash3(seed as u64, k, 0, 7);
                    let h2 = litho_core::hash::splitmix64(h);
                    (
                        (litho_core::hash::unit(h) - 0.5) * 200.0 * l,
                        (litho_core::hash::unit(h2) - 0.5) * 200.0 * l,
                    )
                };
                let s = generator.sample(x, z).unwrap();
                write!(
                    out,
                    "{seed} {} {:016x} {:016x}",
                    mode.as_str(),
                    x.to_bits(),
                    z.to_bits()
                )
                .unwrap();
                for field in &Field::ALL[..5] {
                    write!(
                        out,
                        " {:016x}",
                        generator.field(*field, x, 0.0, z).to_bits()
                    )
                    .unwrap();
                }
                for value in [
                    s.climate.surface.e_km,
                    s.climate.precip.p_mm,
                    s.climate.temperature.t_bio,
                    generator.spawn_shift().x,
                    generator.spawn_shift().z,
                ] {
                    write!(out, " {:016x}", value.to_bits()).unwrap();
                }
                writeln!(out, " {:?}", s.zone).unwrap();
            }
        }
    }
    out
}

#[cfg(not(test))]
fn main() {
    print!("{}", snapshot());
}
