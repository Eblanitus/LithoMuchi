//! К3 (Д-105): крутые края выреза под воду. Идём по прямым линиям и ищем
//! соседние точки, где поверхность у выреза меняется обрывом, и точки, где
//! вырез поднял поверхность выше суши («стол»).

use litho_core::Generator;

/// Шаг пробы вдоль линии, блоки.
const STEP: i64 = 2;
/// Длина линии, блоки.
const LENGTH: i64 = 16_384;
/// Смещения линий от центра, блоки. Линии идут по обеим осям.
const OFFSETS: [i64; 4] = [-6000, -2000, 2000, 6000];
/// Обрыв: перепад поверхности между соседними пробами, блоки.
const CLIFF: f64 = 4.0;

#[derive(Default)]
struct Tally {
    points: u64,
    /// Точки с вырезом под воду (сила > 0).
    cut: u64,
    /// Из них поверхность выше суши до выреза больше чем на полблока.
    raised: u64,
    max_raise: f64,
    max_raise_at: (f64, f64),
    /// Пары соседей с обрывом, где хотя бы у одного есть вырез.
    cliffs: u64,
    /// Из них у точки с вырезом дно выше суши.
    cliffs_raised: u64,
    max_cliff: f64,
    max_cliff_at: (f64, f64),
}

fn line(generator: &Generator, tally: &mut Tally, at: impl Fn(i64) -> (f64, f64)) {
    // (поверхность, сила выреза, дно выше суши)
    let mut previous: Option<(f64, f64, bool)> = None;
    for k in 0..LENGTH / STEP {
        let (x, z) = at(k * STEP - LENGTH / 2);
        let Some(world) = generator.world_sample(x, z) else {
            previous = None;
            continue;
        };
        let h = world.height;
        tally.points += 1;
        let above = h.water_strength > 0.0 && h.bed_y > h.land_y;
        if h.water_strength > 0.0 {
            tally.cut += 1;
            let raise = h.surface_y - h.land_y;
            if raise > 0.5 {
                tally.raised += 1;
            }
            if raise > tally.max_raise {
                tally.max_raise = raise;
                tally.max_raise_at = (x, z);
            }
        }
        if let Some((surface, strength, was_above)) = previous {
            let drop = (h.surface_y - surface).abs();
            if drop >= CLIFF && (h.water_strength > 0.0 || strength > 0.0) {
                tally.cliffs += 1;
                tally.cliffs_raised += u64::from(above || was_above);
                if drop > tally.max_cliff {
                    tally.max_cliff = drop;
                    tally.max_cliff_at = (x, z);
                }
            }
        }
        previous = Some((h.surface_y, h.water_strength, above));
    }
}

/// Профиль через точку: 25 проб по X с шагом 4 блока.
pub fn profile(generator: &Generator, at: (f64, f64)) {
    println!("walls: профиль по X через {:.0}, {:.0}", at.0, at.1);
    println!("       x  поверхность  суша  дно  зеркало  сила  тип");
    for k in -12..=12 {
        let x = at.0 + f64::from(k * 4);
        match generator.world_sample(x, at.1) {
            Some(world) => {
                let h = world.height;
                println!(
                    "  {x:6.0}  {:8.1}  {:6.1}  {:5.1}  {:6.1}  {:4.2}  {}",
                    h.surface_y,
                    h.land_y,
                    h.bed_y,
                    h.water_y,
                    h.water_strength,
                    world.hydrology.kind.as_str()
                );
            }
            None => println!("  {x:6.0}  нет плиты"),
        }
    }
}

pub fn run(generator: &Generator, center: (f64, f64)) -> Result<(), String> {
    let mut tally = Tally::default();
    for offset in OFFSETS {
        line(generator, &mut tally, |t| {
            (center.0 + t as f64, center.1 + offset as f64)
        });
        line(generator, &mut tally, |t| {
            (center.0 + offset as f64, center.1 + t as f64)
        });
    }
    if tally.points == 0 {
        return Err("на линиях нет точек мира".into());
    }
    println!(
        "walls: {n} линий по {LENGTH} блоков, шаг {STEP}; точек с вырезом под воду {} из {}",
        tally.cut,
        tally.points,
        n = 2 * OFFSETS.len()
    );
    println!(
        "  вырез поднял поверхность выше суши: {} точек, самое большое {:.1} блока в {:.0}, {:.0}",
        tally.raised, tally.max_raise, tally.max_raise_at.0, tally.max_raise_at.1
    );
    println!(
        "  обрывы у выреза (≥ {CLIFF} блока на {STEP} блока): {}, из них дно выше суши: {}; самый большой {:.1} блока в {:.0}, {:.0}",
        tally.cliffs,
        tally.cliffs_raised,
        tally.max_cliff,
        tally.max_cliff_at.0,
        tally.max_cliff_at.1
    );
    Ok(())
}
