//! К11 / Д-119 (Ш1): счётчик «биом против пояса».
//!
//! Правило автора: «пояс и есть температура». Пояс П0–П6 считается по средней
//! годовой температуре, и биом суши обязан стоять в одной из допустимых строк
//! ванильной температуры этого пояса:
//!
//! | Пояс | Строки T |
//! |---|---|
//! | П0 полярный, П1 субполярный | t0 |
//! | П2 бореальный | t1 |
//! | П3 прохладный умеренный | t2 |
//! | П4 тёплый умеренный | t2…t4 |
//! | П5 субтропики, П6 тропики | t3…t4 |
//!
//! Печатается доля суши, где биом не из строки своего пояса, с разбивкой по
//! причине: вода (река, болото, котловина, галерея), таблица (пустыня, кайма,
//! сухо) и «имя биома» — видимая ли это разница или только ванильная строка T.
//!
//! Сетка: 240 × 240 проб на квадрате ±64 L — одна клетка полюсов, все широты.
//! Это та же сетка, что у расследования `docs/spec/belts-2026-11-17/`.

use std::collections::BTreeMap;

use litho_core::classify::allowed_rows;
use litho_core::vanilla_biome::{is_cold_mountain, is_mountain, surface_biome};
use litho_core::{Generator, SurfaceKind};

/// Проб на сторону квадрата (`beltprobe`).
const GRID: i64 = 240;
/// Полусторона квадрата в L: одна клетка сетки полюсов.
const HALF_SPAN_L: f64 = 64.0;

/// Имена биомов, которые в игре стоят на каждой ванильной строке T.
fn names_for_row(t: u8) -> &'static [&'static str] {
    match t {
        0 => &[
            "snowy_plains",
            "snowy_taiga",
            "taiga",
            "frozen_river",
            "snowy_beach",
            "ice_spikes",
            "frozen_ocean",
        ],
        1 => &[
            "plains",
            "forest",
            "taiga",
            "old_growth_spruce_taiga",
            "river",
            "swamp",
            "meadow",
        ],
        2 => &[
            "flower_forest",
            "plains",
            "forest",
            "birch_forest",
            "dark_forest",
            "river",
            "swamp",
        ],
        3 => &[
            "savanna",
            "forest",
            "jungle",
            "river",
            "mangrove_swamp",
            "sparse_jungle",
            "bamboo_jungle",
        ],
        _ => &["desert", "river", "mangrove_swamp", "badlands"],
    }
}

/// Видна ли разница по имени биома: имя не встречается ни в одной допустимой
/// строке пояса. Если встречается — расхождение только в ванильной строке T.
fn visible_bad(belt: u8, name: &str) -> bool {
    let (lo, hi) = allowed_rows(belt);
    !(lo..=hi).any(|t| names_for_row(t).contains(&name))
}

fn is_river(name: &str) -> bool {
    name == "river" || name == "frozen_river"
}

#[derive(Default)]
struct Tally {
    land: u64,
    ocean: u64,
    violations: u64,
    violations_visible: u64,
    by_cause: BTreeMap<String, u64>,
    /// Матрица «пояс × итоговая строка T».
    matrix: [[u64; 5]; 7],
    belts: [u64; 7],
    examples: BTreeMap<String, Vec<(f64, f64, f64, &'static str)>>,
    rivers_cold: u64,
    rivers: u64,
    /// Ванильные горные биомы ниже порога `mountain_biome_y`.
    mountain_low: u64,
    /// Из них снежные при неполярной строке T.
    cold_mountain_low: u64,
    mountain_high: u64,
    snow_rows_warm: u64,
}

impl Tally {
    fn example(&mut self, key: &str, x: f64, z: f64, y: f64, biome: &'static str) {
        let entry = self.examples.entry(key.to_string()).or_default();
        if entry.len() < 4 {
            entry.push((x, z, y, biome));
        }
    }
}

/// Гейт CI (Ш6): L1 ≤ 1 % суши и ни одной замёрзшей реки в П2–П4.
const GATE_L1_PCT: f64 = 1.0;

/// Сводка одного прогона: из неё собираются и подробный вывод, и строка Ш8.
#[derive(Clone, Copy)]
pub struct Summary {
    pub land: u64,
    pub l1_pct: f64,
    pub l1_visible_pct: f64,
    pub rivers_cold: u64,
    pub rivers: u64,
    pub mountain_low_pct: f64,
    pub cold_mountain_low_pct: f64,
    pub belts: [f64; 7],
}

pub fn run(generator: &Generator, gate: bool) -> Result<(), String> {
    let tally = count(generator)?;
    print_report(generator, &tally);
    if gate {
        check_gate(&summary(&tally))?;
        println!("К11: гейт пройден (L1 ≤ {GATE_L1_PCT} %, лёд только в t0)");
    }
    Ok(())
}

/// Ш8: все сиды × оба режима одной командой — строка на прогон, в конце
/// сводка долей поясов по режимам и гейт (как в CI, но шире).
pub fn run_sweep(
    seeds: &[i64],
    modes: &[litho_core::Mode],
    version: litho_core::GenVersion,
) -> Result<(), String> {
    let mut failures = Vec::new();
    let mut per_mode: Vec<(litho_core::Mode, Vec<Summary>)> = Vec::new();
    for mode in modes {
        let mut rows = Vec::new();
        for &seed in seeds {
            let world_gen = crate::tune::generator(seed, *mode, version)?;
            let tally = count(&world_gen)?;
            let s = summary(&tally);
            println!(
                "  сид {seed:>12} {:>10}: суша {:>6}  L1 {:5.2} % (по имени {:5.2} %)  лёд П2–П4 {:>4} из {:<5}  горы<y {:.2} % (снежные {:.2} %)  П0…П6 {}",
                mode.as_str(),
                s.land,
                s.l1_pct,
                s.l1_visible_pct,
                s.rivers_cold,
                s.rivers,
                s.mountain_low_pct,
                s.cold_mountain_low_pct,
                s.belts
                    .iter()
                    .map(|p| format!("{p:.1}"))
                    .collect::<Vec<_>>()
                    .join("/")
            );
            if let Err(e) = check_gate(&s) {
                failures.push(format!("сид {seed} {}: {e}", mode.as_str()));
            }
            rows.push(s);
        }
        per_mode.push((*mode, rows));
    }
    println!("К11/Ш8: доли поясов по режиму, % суши (мин…макс по сидам):");
    for (mode, rows) in &per_mode {
        let cells: Vec<String> = (0..7)
            .map(|b| {
                let lo = rows.iter().map(|s| s.belts[b]).fold(f64::MAX, f64::min);
                let hi = rows.iter().map(|s| s.belts[b]).fold(f64::MIN, f64::max);
                format!("П{b} {lo:.1}…{hi:.1}")
            })
            .collect();
        let l1_hi = rows.iter().map(|s| s.l1_pct).fold(f64::MIN, f64::max);
        println!(
            "  {}: {}; L1 максимум {l1_hi:.2} %",
            mode.as_str(),
            cells.join(", ")
        );
    }
    if failures.is_empty() {
        println!("К11/Ш8: гейт пройден на всех прогонах");
        Ok(())
    } else {
        for f in &failures {
            println!("  ГЕЙТ: {f}");
        }
        Err(format!("К11/Ш8: гейт упал на {} прогонах", failures.len()))
    }
}

fn count(generator: &Generator) -> Result<Tally, String> {
    let params = generator.params();
    let half = HALF_SPAN_L * params.block.l_blocks;
    let step = 2.0 * half / GRID as f64;
    let mut tally = Tally::default();
    for j in 0..GRID {
        for i in 0..GRID {
            let x = -half + (i as f64 + 0.5) * step;
            let z = -half + (j as f64 + 0.5) * step;
            let Some(s) = generator.sample(x, z) else {
                continue;
            };
            let world = &s.world;
            if world.is_ocean {
                tally.ocean += 1;
                continue;
            }
            let class = world.class;
            let belt = class.mean_belt.min(6);
            let t = class.cell.t;
            let biome = class.cell.biome;
            let y = world.height.surface_y;
            tally.land += 1;
            tally.belts[belt as usize] += 1;
            tally.matrix[belt as usize][t.min(4) as usize] += 1;
            let (lo, hi) = allowed_rows(belt);
            if !(lo..=hi).contains(&t) {
                tally.violations += 1;
                let visible = visible_bad(belt, biome);
                if visible {
                    tally.violations_visible += 1;
                }
                let cause = match world.hydrology.kind {
                    SurfaceKind::Land => "таблица",
                    _ => "вода",
                };
                let dir = if t > hi {
                    "жарче"
                } else {
                    "холоднее"
                };
                let key = format!("{dir} | {cause} | П{belt} → {biome}");
                tally.example(&key, x, z, y, biome);
            }
            if is_river(biome) && (2..=4).contains(&belt) {
                tally.rivers += 1;
                if biome == "frozen_river" {
                    tally.rivers_cold += 1;
                }
            }
            // Д-118: горные биомы ванили на холмах.
            let v = world.vanilla;
            let game = surface_biome(v.c, v.e, v.w, v.t, v.h);
            if is_mountain(game) {
                if y < params.climate.mountain_biome_y {
                    tally.mountain_low += 1;
                    if is_cold_mountain(game) && t != 0 {
                        tally.cold_mountain_low += 1;
                    }
                } else {
                    tally.mountain_high += 1;
                    if is_cold_mountain(game) && t >= 2 {
                        tally.snow_rows_warm += 1;
                    }
                }
            }
        }
    }
    if tally.land == 0 {
        return Err("на сетке нет суши".into());
    }
    Ok(tally)
}

fn print_report(generator: &Generator, tally: &Tally) {
    let params = generator.params();
    let step = 2.0 * HALF_SPAN_L * params.block.l_blocks / GRID as f64;
    let n = tally.land as f64;
    let pct = |count: u64| 100.0 * count as f64 / n;
    println!(
        "К11 биом против пояса: seed={} mode={} version={}",
        generator.seed() as i64,
        params.mode.as_str(),
        params.version.as_str()
    );
    println!(
        "  суша {} проб, океан {}, шаг {:.0} блоков, квадрат ±{} L",
        tally.land, tally.ocean, step, HALF_SPAN_L
    );
    println!(
        "  пояс по средней температуре, % суши: {}",
        (0..7)
            .map(|b| format!("П{b} {:.1}", pct(tally.belts[b])))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  L1 — биом не из строки своего пояса: {:.2} % суши (видимо по имени {:.2} %)",
        pct(tally.violations),
        pct(tally.violations_visible)
    );
    let mut causes: Vec<_> = tally.by_cause.iter().collect();
    causes.sort_by(|a, b| b.1.cmp(a.1));
    for (key, count) in causes.iter().take(15) {
        println!("    {:6.2} %  {key}", pct(**count));
        if let Some(list) = tally.examples.get(*key) {
            for (x, z, y, biome) in list.iter().take(2) {
                println!("             напр. X {x:.0} Z {z:.0} (y {y:.0}): {biome}");
            }
        }
    }
    println!(
        "  замёрзших рек в П2–П4: {:.0} % ({} из {})",
        100.0 * tally.rivers_cold as f64 / tally.rivers.max(1) as f64,
        tally.rivers_cold,
        tally.rivers
    );
    println!(
        "  горные биомы игры ниже y {:.0}: {:.2} % суши (из них снежные при строке T > 0: {:.2} %)",
        params.climate.mountain_biome_y,
        pct(tally.mountain_low),
        pct(tally.cold_mountain_low)
    );
    println!(
        "  горные биомы игры от y {:.0}: {:.2} % суши, из них снежные при строке T ≥ 2: {:.2} %",
        params.climate.mountain_biome_y,
        pct(tally.mountain_high),
        pct(tally.snow_rows_warm)
    );
    println!("  матрица П(пояс) × t(итог), % суши:");
    for b in 0..7_u8 {
        let row: Vec<String> = (0..5_u8)
            .map(|t| format!("{:5.1}", pct(tally.matrix[b as usize][t as usize])))
            .collect();
        println!("    П{b}: t0..t4 = {}", row.join(" "));
    }
}

fn summary(tally: &Tally) -> Summary {
    let n = tally.land.max(1) as f64;
    let pct = |count: u64| 100.0 * count as f64 / n;
    Summary {
        land: tally.land,
        l1_pct: pct(tally.violations),
        l1_visible_pct: pct(tally.violations_visible),
        rivers_cold: tally.rivers_cold,
        rivers: tally.rivers,
        mountain_low_pct: pct(tally.mountain_low),
        cold_mountain_low_pct: pct(tally.cold_mountain_low),
        belts: std::array::from_fn(|b| pct(tally.belts[b])),
    }
}

fn check_gate(s: &Summary) -> Result<(), String> {
    if s.l1_pct > GATE_L1_PCT {
        return Err(format!(
            "К11: L1 {:.2} % суши при пороге {GATE_L1_PCT} %",
            s.l1_pct
        ));
    }
    if s.rivers_cold > 0 {
        return Err(format!(
            "К11: замёрзших рек в П2–П4 {} из {}",
            s.rivers_cold, s.rivers
        ));
    }
    Ok(())
}
