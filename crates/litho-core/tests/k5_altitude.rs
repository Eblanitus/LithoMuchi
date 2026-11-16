//! К5 (Д-111): вычет температуры считается по высоте поверхности в игре,
//! а не по модельной высоте коры. Проверки идут по линиям настоящего мира.

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
    // Вычет идёт по крупной форме, без мелких бугров; запас — их размах.
    let bumps = p
        .height
        .micro_amp_blocks
        .iter()
        .fold(0.0_f64, |a, b| a.max(*b))
        * 3.0;
    let points = dry_land(&g, &LINES);
    assert!(points.len() > 500, "мало суши: {}", points.len());

    // Холмы ниже порога пояс не меняют: вычет там один и тот же на всей
    // суше, хотя модельная высота под ними разная.
    let low: Vec<_> = points.iter().filter(|q| q.0 < base - bumps).collect();
    assert!(low.len() > 300);
    let worst = low
        .iter()
        .map(|q| (q.1 - flat_c).abs())
        .fold(0.0_f64, f64::max);
    assert!(worst < 1e-9, "вычет ниже порога гуляет на {worst} °C");
    let old_spread = low.iter().map(|q| q.2).fold(0.0_f64, f64::max)
        - low.iter().map(|q| q.2).fold(f64::INFINITY, f64::min);
    assert!(
        old_spread > 3.0,
        "прежний вычет на низкой суше гулял на {old_spread:.1} °C"
    );

    // Каждая точка: вычет следует высоте поверхности.
    for q in &points {
        let want = flat_c + rate * (q.0 - base).max(0.0);
        assert!(
            (q.1 - want).abs() <= rate * bumps + 1e-9,
            "y {:.1}: вычет {:.2} °C, по высоте {:.2}",
            q.0,
            q.1,
            want
        );
    }

    // Горы (от y 130) холодны как раньше: средний вычет в пределах
    // от половины до полутора прежнего.
    let high: Vec<_> = points.iter().filter(|q| q.0 >= 130.0).collect();
    assert!(high.len() > 20, "мало гор: {}", high.len());
    let now = high.iter().map(|q| q.1).sum::<f64>() / high.len() as f64;
    let before = high.iter().map(|q| q.2).sum::<f64>() / high.len() as f64;
    assert!(
        now > 0.5 * before && now < 1.5 * before,
        "горы: вычет {now:.1} °C, прежний {before:.1} °C"
    );
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
