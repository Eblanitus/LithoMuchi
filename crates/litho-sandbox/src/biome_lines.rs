//! К1 (Д-103): как часто меняется биом вдоль прямых линий и какого размера
//! пятна. Считается по суше: океан и точки без плиты линию разрывают.

use litho_core::Generator;

/// Шаг пробы вдоль линии, блоки: ячейка биома в игре 4 × 4 блока.
const STEP: i64 = 4;
/// Длина линии, блоки.
const LENGTH: i64 = 16_384;
/// Смещения линий от центра, блоки. Линии идут по обеим осям.
const OFFSETS: [i64; 4] = [-6000, -2000, 2000, 6000];

#[derive(Default)]
struct Tally {
    /// Пары соседних проб суши.
    pairs: u64,
    biome: u64,
    belt: u64,
    province: u64,
    /// Длины пятен одного биома, блоки (пятно ограничено сменой биома).
    runs: Vec<i64>,
}

fn line(generator: &Generator, tally: &mut Tally, at: impl Fn(i64) -> (f64, f64)) {
    let mut previous: Option<(&'static str, u8, u8)> = None;
    let mut run = 0_i64;
    // Пятно считается, только если оно с обеих сторон ограничено сменой
    // биома, а не берегом или концом линии.
    let mut closed_start = false;
    for k in 0..LENGTH / STEP {
        let (x, z) = at(k * STEP - LENGTH / 2);
        let current = generator
            .sample(x, z)
            .filter(|sample| !sample.class.is_ocean)
            .map(|sample| (sample.cell.biome, sample.zone.belt, sample.zone.province));
        match (previous, current) {
            (Some(a), Some(b)) => {
                tally.pairs += 1;
                tally.belt += u64::from(a.1 != b.1);
                tally.province += u64::from(a.2 != b.2);
                if a.0 == b.0 {
                    run += STEP;
                } else {
                    tally.biome += 1;
                    if closed_start {
                        tally.runs.push(run);
                    }
                    run = STEP;
                    closed_start = true;
                }
            }
            (None, Some(_)) => {
                run = STEP;
                closed_start = false;
            }
            _ => {}
        }
        previous = current;
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
    if tally.pairs == 0 || tally.biome == 0 {
        return Err("на линиях нет суши или нет ни одной смены биома".into());
    }
    let land = tally.pairs as f64 * STEP as f64;
    let every = |changes: u64| {
        if changes == 0 {
            f64::INFINITY
        } else {
            land / changes as f64
        }
    };
    tally.runs.sort_unstable();
    let quantile = |q: f64| {
        if tally.runs.is_empty() {
            0
        } else {
            tally.runs[((tally.runs.len() - 1) as f64 * q).round() as usize]
        }
    };
    println!(
        "biomes: {n} линий по {LENGTH} блоков, шаг {STEP}; суши {land:.0} блоков",
        n = 2 * OFFSETS.len()
    );
    println!(
        "  смена биома раз в {:.0} блоков ({} смен)",
        every(tally.biome),
        tally.biome
    );
    println!(
        "  смена пояса раз в {:.0} блоков; провинции раз в {:.0} блоков",
        every(tally.belt),
        every(tally.province)
    );
    println!(
        "  пятно биома: медиана {} блоков, четверти {} и {} ({} пятен)",
        quantile(0.5),
        quantile(0.25),
        quantile(0.75),
        tally.runs.len()
    );
    Ok(())
}
