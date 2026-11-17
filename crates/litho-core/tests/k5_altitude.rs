//! К5 (Д-111) → К11 (Д-119): вычет температуры суши одинаков на всей суше и
//! от высоты поверхности не зависит; высота работает только в горной лестнице
//! биомов (Д-118). Проверки идут по линиям настоящего мира.

use litho_core::climate::{temperature, temperature_cooled};
use litho_core::version::GenVersion;
use litho_core::{Generator, Mode};

/// Сухая точка суши: (высота поверхности, вычет за высоту, прежний вычет).
fn dry_land(g: &Generator, lines: &[(f64, bool)]) -> Vec<(f64, f64, f64)> {
    let p = g.params();
    let mut out = Vec::new();
    for &(offset, along_x) in lines {
        for k in -1024..1024 {
            let t = f64::from(k) * 8.0;
            let (x, z) = if along_x { (t, offset) } else { (offset, t) };
            let Some(s) = g.sample(x, z) else { continue };
            if s.world.is_ocean || s.world.river > 0.0 {
                continue;
            }
            let c = &s.world.climate;
            let flat = temperature_cooled(c.pole.phi_deg, 0.0, c.precip.cont, p).t_c;
            let old = temperature(c.pole.phi_deg, c.surface.e_km, c.precip.cont, p).t_c;
            out.push((
                s.world.height.surface_y,
                flat - c.temperature.t_c,
                flat - old,
            ));
        }
    }
    out
}

const LINES: [(f64, bool); 4] = [
    (-6000.0, true),
    (2000.0, true),
    (-2000.0, false),
    (6000.0, false),
];

fn check(seed: i64, mode: Mode) {
    let g = Generator::new(seed, mode, GenVersion::V1Dev).expect("мир v1-dev");
    let p = g.params();
    let base = p.climate.alt_cool_base_y;
    let rate = p.climate.alt_cool_c_per_block;
    let flat_c = p.climate.alt_cool_land_c;
    // К11 / Д-119: высота пояс больше не двигает. Вычет один на всей суше —
    // ровно `alt_cool_land_c`, и на холме, и на горе.
    assert_eq!(rate, 0.0, "v1-dev: вычет по высоте должен быть выключен");
    let points = dry_land(&g, &LINES);
    assert!(points.len() > 500, "мало суши: {}", points.len());
    let worst = points
        .iter()
        .map(|q| (q.1 - flat_c).abs())
        .fold(0.0_f64, f64::max);
    assert!(worst < 1e-9, "вычет суши гуляет на {worst} °C");
    // Прежний вычет по модельной коре рос с высотой — на нём и держался К5.
    let old_spread = points.iter().map(|q| q.2).fold(0.0_f64, f64::max)
        - points.iter().map(|q| q.2).fold(f64::INFINITY, f64::min);
    assert!(
        old_spread > 3.0,
        "прежний вычет гулял на {old_spread:.1} °C"
    );

    // Горы различаются высотой, но не вычетом: у самой высокой точки он тот
    // же, что у подножия. Холод на горе даёт ванильная горная лестница
    // (`mountain_biome_y`, Д-118), а не смена пояса.
    let highest = points
        .iter()
        .fold(None::<&(f64, f64, f64)>, |acc, q| match acc {
            Some(best) if best.0 >= q.0 => Some(best),
            _ => Some(q),
        })
        .copied()
        .expect("суша есть");
    assert!(highest.0 > base, "нет гор выше порога: {}", highest.0);
    assert!(
        (highest.1 - flat_c).abs() < 1e-9,
        "на горе y {:.0} вычет {:.2} °C вместо {flat_c:.2}",
        highest.0,
        highest.1
    );
    let high: Vec<_> = points.iter().filter(|q| q.0 >= base).collect();
    if !high.is_empty() {
        let now = high.iter().map(|q| q.1).sum::<f64>() / high.len() as f64;
        let before = high.iter().map(|q| q.2).sum::<f64>() / high.len() as f64;
        assert!(
            (now - flat_c).abs() < 1e-9,
            "горы: вычет {now:.2} °C вместо {flat_c:.2} (было по высоте {before:.1} °C)"
        );
    }
}

#[test]
fn cooling_follows_game_height_gameplay() {
    check(42, Mode::Gameplay);
}

#[test]
fn cooling_follows_game_height_scientific() {
    check(42, Mode::Scientific);
}

/// `v0` заморожен: вычет прежний, 6,5 °C на км модельной высоты.
#[test]
fn v0_keeps_lapse_by_model_height() {
    let g = Generator::new(42, Mode::Gameplay, GenVersion::V0).expect("мир v0");
    let p = g.params();
    let mut seen = 0;
    for k in -200..200 {
        let Some(s) = g.sample(f64::from(k) * 40.0, 2000.0) else {
            continue;
        };
        let c = &s.world.climate;
        let old = temperature(c.pole.phi_deg, c.surface.e_km, c.precip.cont, p).t_c;
        assert!(
            (c.temperature.t_c - old).abs() < 0.5,
            "v0 сменил температуру"
        );
        seen += 1;
    }
    assert!(seen > 100);
}
