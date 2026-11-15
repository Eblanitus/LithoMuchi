//! К4а/часть 2 (Д-110): крутизна своей поверхности. Проверки идут по
//! линиям настоящего мира `v1-dev`, в местах, где замер `walls` находил стены.

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

/// Наибольший перепад высоты между соседними блоками линии.
fn worst_step(g: &Generator, from: (i64, i64), to: (i64, i64)) -> f64 {
    let n = (to.0 - from.0).abs().max((to.1 - from.1).abs());
    let (dx, dz) = ((to.0 - from.0).signum(), (to.1 - from.1).signum());
    let ys: Vec<f64> = (0..=n)
        .map(|k| {
            let (x, z) = ((from.0 + dx * k) as f64, (from.1 + dz * k) as f64);
            g.sample(x, z).expect("точка мира").world.height.surface_y
        })
        .collect();
    ys.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0_f64, f64::max)
}

/// Д-110: предгорья гасли на M 0,4…0,5 — в gameplay это 20 блоков, а горная
/// E там ещё не набрана. Сид 42, X 3976…4006, Z 2000: было 114 → 84 за
/// 8 блоков (ров за предгорьем).
#[test]
fn foothills_do_not_end_in_a_moat() {
    let g = world(42, Mode::Gameplay);
    let worst = worst_step(&g, (3960, 2000), (4020, 2000));
    assert!(worst <= 3.0, "перепад {worst:.1} блока на блок");
}

/// Д-110: перекладка E аридной зоны в scientific растягивала отрезок
/// 0,04…0,05 в 27 раз. Сид 7, X 2000, Z 12926: было 15 блоков на 2.
#[test]
fn arid_remap_has_no_wall_in_scientific() {
    let g = world(7, Mode::Scientific);
    let worst = worst_step(&g, (2000, 12900), (2000, 12950));
    assert!(worst <= 3.0, "перепад {worst:.1} блока на блок");
}

/// Д-110: расстояние до берега — настоящее расстояние до ячеек кромки:
/// между соседними блоками оно меняется не больше чем на блок и у воды
/// доходит до нуля. Прежний марш давал ступени по 4 блока и терял берег.
#[test]
fn shore_distance_changes_by_at_most_a_block_per_block() {
    let g = world(7, Mode::Gameplay);
    // Берег у (2000, 8170): линия поперёк него.
    let line: Vec<f64> = (8060..=8260)
        .map(|z| g.shore_distance_blocks(2000.0, f64::from(z), 1.0))
        .collect();
    let worst = line
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0_f64, f64::max);
    assert!(
        worst <= 1.0 + 1e-9,
        "расстояние прыгает на {worst:.2} блока"
    );
    let nearest = line.iter().copied().fold(f64::INFINITY, f64::min);
    let farthest = line.iter().copied().fold(0.0_f64, f64::max);
    assert!(nearest < 2.0, "линия не дошла до воды: {nearest:.1}");
    assert!(farthest > 24.0, "линия не ушла от берега: {farthest:.1}");
}

/// Д-110: спуск к воде на той же линии — без лестницы.
#[test]
fn shore_ramp_is_not_a_staircase() {
    let g = world(7, Mode::Gameplay);
    let worst = worst_step(&g, (2000, 8060), (2000, 8260));
    assert!(worst <= 3.0, "перепад {worst:.1} блока на блок");
}
