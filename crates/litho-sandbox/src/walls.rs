//! К3/К4а: счётчик крутых ступеней на прямых линиях.
//!
//! Ступень — перепад больше 3 блоков на блок между соседними пробами шагом
//! 2 блока (как в разборе Д-106). Считаются только пары, где обе точки — суша
//! выше моря: океан и берег линии разрывают. Ступени разложены по причинам,
//! которые видно снаружи ядра:
//! - у выреза воды (`height.water_strength`): край русла/котловины;
//! - из них «стол со стеной» — вырезанная точка стоит выше суши рядом
//!   (баг К3: дно озера выше берега);
//! - у реки (`river`);
//! - суша вдали от воды (то, что разбирает К4а, часть 2).

use litho_core::height::SEA_LEVEL;
use litho_core::Generator;

/// Шаг пробы вдоль линии, блоки.
const STEP: i64 = 2;
/// Длина линии, блоки.
const LENGTH: i64 = 16_384;
/// Смещения линий от центра, блоки: те же, что у `biomes` (К1, Д-103).
const OFFSETS: [i64; 4] = [-6000, -2000, 2000, 6000];
/// Порог ступени: перепад больше 3 блоков на блок.
const STEEP_PER_BLOCK: f64 = 3.0;

/// Проба линии: только то, что нужно счётчику.
#[derive(Clone, Copy)]
struct Point {
    y: f64,
    /// Суша выше моря: обе такие точки дают пару.
    land: bool,
    /// Сила выреза под воду (`height.water_strength`).
    cut: f64,
    /// Вода отложена до R2 (`DeferredWater`).
    deferred: f64,
    /// Геометрическая вода озера/болота/внутреннего моря.
    lake: f64,
    river: f64,
}

impl Point {
    fn near_water(self) -> bool {
        self.cut > 0.0 || self.deferred > 0.0 || self.lake > 0.0
    }
}

#[derive(Default)]
struct Tally {
    pairs: u64,
    steep: u64,
    /// Крутые края выреза воды: одна точка вырезана, вторая — нет.
    water_edge: u64,
    /// Из них «стол со стеной»: вырезанная точка выше суши рядом.
    raised: u64,
    /// Худший подъём выреза над сушей рядом, блоки, и где он.
    worst_raise: (f64, f64, f64),
    /// Крутые ступени у реки (выреза нет).
    river: u64,
    /// Крутые ступени на суше вдали от воды.
    dry: u64,
    /// Самая крутая ступень: перепад, x, z.
    worst: (f64, f64, f64),
}

fn point(generator: &Generator, x: f64, z: f64) -> Option<Point> {
    let sample = generator.sample(x, z)?;
    let height = &sample.world.height;
    let hydrology = &sample.world.hydrology;
    let ocean = sample.class.is_ocean;
    Some(Point {
        y: height.surface_y,
        land: !ocean && height.surface_y > SEA_LEVEL,
        cut: height.water_strength,
        deferred: hydrology.r2_deferred,
        lake: hydrology.lakes.water,
        river: sample.world.river,
    })
}

fn line(generator: &Generator, tally: &mut Tally, at: impl Fn(i64) -> (f64, f64)) {
    let mut previous: Option<(Point, f64, f64)> = None;
    for k in 0..LENGTH / STEP {
        let t = k * STEP - LENGTH / 2;
        let (x, z) = at(t);
        let current = point(generator, x, z).map(|p| (p, x, z));
        let (b, bx, bz) = match current {
            Some(c) => c,
            None => {
                previous = None;
                continue;
            }
        };
        if let Some((a, ax, az)) = previous {
            if a.land && b.land {
                tally.pairs += 1;
                let dy = (b.y - a.y).abs();
                if dy > STEEP_PER_BLOCK * STEP as f64 {
                    tally.steep += 1;
                    if dy > tally.worst.0 {
                        tally.worst = (dy, bx, bz);
                    }
                    let cut_a = a.cut > 0.0;
                    let cut_b = b.cut > 0.0;
                    if cut_a != cut_b {
                        tally.water_edge += 1;
                        // Подъём считаем по вырезанной точке: она должна быть
                        // ниже суши рядом (К3).
                        let (cut, land) = if cut_a { (a.y, b.y) } else { (b.y, a.y) };
                        let raise = cut - land;
                        if raise > 1.0 {
                            tally.raised += 1;
                            if raise > tally.worst_raise.0 {
                                tally.worst_raise = (raise, if cut_a { ax } else { bx }, if cut_a { az } else { bz });
                            }
                        }
                    } else if a.river > 0.0 || b.river > 0.0 {
                        tally.river += 1;
                    } else if !a.near_water() && !b.near_water() {
                        tally.dry += 1;
                    }
                }
            }
        }
        previous = Some((b, bx, bz));
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
    if tally.pairs == 0 {
        return Err("на линиях нет суши выше моря".into());
    }
    let per = |n: u64| 100.0 * n as f64 / tally.pairs as f64;
    let land = tally.pairs as f64 * STEP as f64;
    println!(
        "walls: {n} линий по {LENGTH} блоков, шаг {STEP}; суши выше моря {land:.0} блоков ({pairs} пар)",
        n = 2 * OFFSETS.len(),
        pairs = tally.pairs
    );
    println!(
        "  ступеней круче {STEEP_PER_BLOCK} блоков на блок: {} ({:.3} % пар), это раз в {:.0} блоков",
        tally.steep,
        per(tally.steep),
        if tally.steep == 0 {
            f64::INFINITY
        } else {
            land / tally.steep as f64
        }
    );
    println!(
        "  у выреза воды: {} ({:.0} % ступеней)",
        tally.water_edge,
        if tally.steep == 0 {
            0.0
        } else {
            100.0 * tally.water_edge as f64 / tally.steep as f64
        }
    );
    println!(
        "    из них дно выше суши рядом («стол со стеной»): {}, худший подъём {:.0} блоков у ({:.0}, {:.0})",
        tally.raised, tally.worst_raise.0, tally.worst_raise.1, tally.worst_raise.2
    );
    println!("  у реки: {}", tally.river);
    println!("  суша вдали от воды: {}", tally.dry);
    println!(
        "  самая крутая ступень: {:.0} блоков на {STEP} блока у ({:.0}, {:.0})",
        tally.worst.0, tally.worst.1, tally.worst.2
    );
    Ok(())
}
