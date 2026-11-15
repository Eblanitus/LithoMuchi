//! К4а/часть 2 (Д-110): швы коры у тройных стыков. Проверки идут по
//! линиям настоящего мира `v1-dev`.

use litho_core::version::GenVersion;
use litho_core::{Generator, Mode};

fn world(seed: i64, mode: Mode) -> Generator {
    Generator::new(seed, mode, GenVersion::V1Dev).expect("мир v1-dev")
}

/// Линия шагом 1 блок: (e_km, mountain) коры в каждой точке.
fn crust_line(g: &Generator, x0: i64, x1: i64, z: f64) -> Vec<(f64, f64)> {
    (x0..=x1)
        .filter_map(|x| g.crust_sample(x as f64, z))
        .map(|c| (c.e_km, c.mountain))
        .collect()
}

/// Д-110: на биссектрисе d_AB = d_AC внутри плиты тип деформации раньше
/// перескакивал с пары A|B на пару A|C. Сид 7, научный режим, X −1821,
/// Z −2000: `e_km` 0,66 → 1,22 км и `mountain` 0,03 → 0,21 за один блок,
/// в игре — стена 50 блоков.
#[test]
fn crust_is_continuous_where_second_and_third_plates_swap() {
    let g = world(7, Mode::Scientific);
    let line = crust_line(&g, -1900, -1740, -2000.0);
    assert!(line.len() > 150);
    let worst_e = line
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).abs())
        .fold(0.0_f64, f64::max);
    let worst_m = line
        .windows(2)
        .map(|w| (w[1].1 - w[0].1).abs())
        .fold(0.0_f64, f64::max);
    assert!(worst_e < 0.02, "скачок e_km за блок: {worst_e:.3} км");
    assert!(worst_m < 0.01, "скачок mountain за блок: {worst_m:.3}");
}

/// Критерий плана К4а: сид 42, геймплейный, вокруг X 20885, Z −915 уклон
/// поверхности не больше 1,5 блока на блок (была стена 36 блоков на 10).
#[test]
fn plan_point_has_no_wall() {
    let g = world(42, Mode::Gameplay);
    let (cx, cz) = (20885.0, -915.0);
    let y = |x: f64, z: f64| g.sample(x, z).expect("точка мира").world.height.surface_y;
    let centre = y(cx, cz);
    let mut worst = 0.0_f64;
    for i in -4..=4 {
        for j in -4..=4 {
            if i == 0 && j == 0 {
                continue;
            }
            let (dx, dz) = (4.0 * f64::from(i), 4.0 * f64::from(j));
            let slope = (y(cx + dx, cz + dz) - centre).abs() / (dx * dx + dz * dz).sqrt();
            worst = worst.max(slope);
        }
    }
    assert!(worst <= 1.5, "уклон {worst:.2} блока на блок");
}
