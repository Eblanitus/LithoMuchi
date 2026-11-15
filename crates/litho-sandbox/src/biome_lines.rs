//! К1 (Д-103): как часто меняется биом вдоль прямых линий и какого размера
//! пятна. Считается по суше: океан и точки без плиты линию разрывают.
//! Реки (`river`, `frozen_river`) считаются и вместе со всеми, и отдельно.

use std::collections::BTreeMap;

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
    /// Смены биома, где с одной из сторон река (`river`, `frozen_river`).
    river: u64,
    belt: u64,
    province: u64,
    /// К2: соседние пробы, у которых ванильная ступень T или H отличается
    /// на 2 и больше. Реки не в счёт: у них T/H климата под ними.
    jumps_t: u64,
    jumps_h: u64,
    /// Пары проб суши без рек, по которым считаются прыжки.
    jump_pairs: u64,
    /// Какие биомы стоят по сторонам прыжка и сколько раз.
    jump_kinds: BTreeMap<(&'static str, &'static str), u64>,
    /// Длины пятен одного биома, блоки (пятно ограничено сменой биома).
    runs: Vec<i64>,
    /// То же без рек: речные пробы пропущены, лес по обе стороны реки —
    /// одно пятно, ширина реки в его длину не входит.
    dry: Runs,
}

/// Счёт пятен вдоль линии. Пятно считается, только если оно с обеих сторон
/// ограничено сменой биома, а не берегом или концом линии.
#[derive(Default)]
struct Runs {
    previous: Option<&'static str>,
    run: i64,
    closed_start: bool,
    changes: u64,
    /// Пробы, вошедшие в счёт (без первой на каждом отрезке суши).
    pairs: u64,
    lengths: Vec<i64>,
}

impl Runs {
    fn push(&mut self, biome: &'static str) {
        match self.previous {
            Some(previous) => {
                self.pairs += 1;
                if previous == biome {
                    self.run += STEP;
                } else {
                    self.changes += 1;
                    if self.closed_start {
                        self.lengths.push(self.run);
                    }
                    self.run = STEP;
                    self.closed_start = true;
                }
            }
            None => {
                self.run = STEP;
                self.closed_start = false;
            }
        }
        self.previous = Some(biome);
    }

    /// Берег или конец линии: незакрытое пятно не считается.
    fn gap(&mut self) {
        self.previous = None;
    }
}

fn is_river(biome: &str) -> bool {
    biome == "river" || biome == "frozen_river"
}

fn line(generator: &Generator, tally: &mut Tally, at: impl Fn(i64) -> (f64, f64)) {
    let mut all = Runs::default();
    tally.dry.gap();
    let mut previous: Option<(&'static str, u8, u8)> = None;
    // Последняя проба суши без реки: (биом, ступень T, ступень H).
    let mut dry_cell: Option<(&'static str, u8, u8)> = None;
    for k in 0..LENGTH / STEP {
        let (x, z) = at(k * STEP - LENGTH / 2);
        let current = generator
            .sample(x, z)
            .filter(|sample| !sample.class.is_ocean)
            .map(|sample| {
                (
                    sample.cell.biome,
                    sample.zone.belt,
                    sample.zone.province,
                    sample.cell.t,
                    sample.cell.h,
                )
            });
        let cell = current.map(|c| (c.0, c.3, c.4));
        let current = current.map(|c| (c.0, c.1, c.2));
        match cell {
            Some(b) if !is_river(b.0) => {
                if let Some(a) = dry_cell {
                    tally.jump_pairs += 1;
                    let jump_t = a.1.abs_diff(b.1) >= 2;
                    let jump_h = a.2.abs_diff(b.2) >= 2;
                    tally.jumps_t += u64::from(jump_t);
                    tally.jumps_h += u64::from(jump_h);
                    if jump_t || jump_h {
                        let key = if a.0 <= b.0 { (a.0, b.0) } else { (b.0, a.0) };
                        *tally.jump_kinds.entry(key).or_default() += 1;
                    }
                }
                dry_cell = Some(b);
            }
            Some(_) => {}
            None => dry_cell = None,
        }
        match current {
            Some(b) => {
                if let Some(a) = previous {
                    tally.pairs += 1;
                    tally.belt += u64::from(a.1 != b.1);
                    tally.province += u64::from(a.2 != b.2);
                    if a.0 != b.0 {
                        tally.biome += 1;
                        tally.river += u64::from(is_river(a.0) || is_river(b.0));
                    }
                }
                all.push(b.0);
                if !is_river(b.0) {
                    tally.dry.push(b.0);
                }
            }
            None => {
                all.gap();
                tally.dry.gap();
            }
        }
        previous = current;
    }
    tally.runs.append(&mut all.lengths);
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
    tally.dry.lengths.sort_unstable();
    let quantile_of = |runs: &[i64], q: f64| {
        if runs.is_empty() {
            0
        } else {
            runs[((runs.len() - 1) as f64 * q).round() as usize]
        }
    };
    let quantile = |q: f64| quantile_of(&tally.runs, q);
    let dry = |q: f64| quantile_of(&tally.dry.lengths, q);
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
    println!(
        "  из-за рек: {} смен из {} ({:.0} %)",
        tally.river,
        tally.biome,
        100.0 * tally.river as f64 / tally.biome as f64
    );
    let dry_land = tally.dry.pairs as f64 * STEP as f64;
    println!(
        "  без рек: смена биома раз в {:.0} блоков ({} смен)",
        if tally.dry.changes == 0 {
            f64::INFINITY
        } else {
            dry_land / tally.dry.changes as f64
        },
        tally.dry.changes
    );
    println!(
        "  без рек: пятно биома медиана {} блоков, четверти {} и {} ({} пятен)",
        dry(0.5),
        dry(0.25),
        dry(0.75),
        tally.dry.lengths.len()
    );
    let per_1000 = |count: u64| 1000.0 * count as f64 / tally.jump_pairs.max(1) as f64;
    println!(
        "  прыжки через ступень: T {} и H {} на {} пар соседей ({:.2} и {:.2} на 1000)",
        tally.jumps_t,
        tally.jumps_h,
        tally.jump_pairs,
        per_1000(tally.jumps_t),
        per_1000(tally.jumps_h)
    );
    let jumps: u64 = tally.jump_kinds.values().sum();
    println!(
        "    всего пар с прыжком {jumps}: {:.1} % смен биома без рек",
        100.0 * jumps as f64 / tally.dry.changes.max(1) as f64
    );
    let mut kinds: Vec<_> = tally.jump_kinds.iter().collect();
    kinds.sort_by(|a, b| b.1.cmp(a.1));
    for ((a, b), count) in kinds.into_iter().take(5) {
        println!("    {a} ↔ {b}: {count}");
    }
    Ok(())
}
