//! Deterministic field survey, NOT a guarantee of a cave or lava block at a point.
//! cargo run -p litho-core --release --example probe_lava -- SEED MODE
use litho_core::{
    GenVersion, Generator, Mode,
    hash::{hash3, unit},
};
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let seed = args
        .first()
        .expect("seed")
        .parse::<i64>()
        .expect("i64 seed");
    let mode = Mode::parse(args.get(1).expect("mode")).expect("gameplay|scientific");
    let g = Generator::new(seed, mode, GenVersion::V0).unwrap();
    let background = g.params().lava.background;
    let mut buckets = [(0, 0); 3];
    let mut hotspots = Vec::new();
    for i in 0..8192 {
        let h = |axis| hash3(seed as u64, i, axis, 97);
        let coord = |hash| ((unit(hash) - 0.5) * 400.0 * mode.l_blocks() / 64.0).floor() * 64.0;
        let x = coord(h(0));
        let z = coord(h(1));
        let heat = g.lava_sample(x, z);
        let active = g.aquifer_lava(x, -40.0, z).abs() > 0.3;
        let bucket = if heat <= background + 1e-9 {
            0
        } else if heat < 0.4 {
            1
        } else {
            2
        };
        buckets[bucket].0 += 1;
        buckets[bucket].1 += usize::from(active);
        if active && heat > background + 0.05 && g.surface_height_km(x, z) > 0.0 {
            hotspots.push((heat, x as i32, z as i32));
        }
    }
    println!(
        "seed={seed} mode={} 8192 sampled aquifer cells; Y cell=-1",
        mode.as_str()
    );
    for (name, (total, active)) in ["background", "warm", "hot"].into_iter().zip(buckets) {
        println!("{name}: {active}/{total} cells above Horizon abs(lava)>0.3 threshold");
    }
    hotspots.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (heat, x, z) in hotspots.into_iter().take(8) {
        println!(
            "active underground FIELD cell x={x} z={z} heat={heat:.4}; /tp @s {} -30 {}",
            x + 32,
            z + 32
        );
    }
    println!(
        "Fluid level, floodedness, cave density and vanilla bottom lava still affect actual blocks."
    );
}
