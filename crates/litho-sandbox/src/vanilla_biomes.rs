//! Д-118: какой биом поставит игра. Две проверки:
//! - `--report ПУТЬ` — сверка копии ванильной таблицы (`litho_core::
//!   vanilla_biome`) с отчётом datagen `biome_parameters/minecraft/
//!   overworld.json` настоящего сервера 26.3 на случайных точках;
//! - без отчёта — по линиям мира: где биом игры не совпадает с биомом
//!   нашего классификатора, и сколько горных биомов стоит на холмах.

use std::collections::BTreeMap;

use litho_core::Generator;
use litho_core::vanilla_biome::{is_cold_mountain, is_mountain, surface_biome};

use crate::json::{self, Json};

const STEP: i64 = 16;
const LENGTH: i64 = 16_384;
const OFFSETS: [i64; 4] = [-6000, -2000, 2000, 6000];

fn is_river(biome: &str) -> bool {
    biome == "river" || biome == "frozen_river"
}

#[derive(Default)]
struct Tally {
    land: u64,
    differ: u64,
    pairs: BTreeMap<(&'static str, &'static str), u64>,
    /// Горные биомы игры (склоны, роща, пики) ниже `mountain_biome_y`.
    mountain_low: u64,
    /// Из них холодные на нашем не снежном биоме — то, что Д-118 запрещает.
    cold_on_warm_low: u64,
    mountain_high: u64,
    examples: Vec<(f64, f64, f64, &'static str, &'static str)>,
}

pub fn run(generator: &Generator, center: (f64, f64)) -> Result<(), String> {
    let params = generator.params();
    let limit = params.climate.mountain_biome_y;
    let mut tally = Tally::default();
    let mut probe = |x: f64, z: f64| {
        let Some(s) = generator.sample(x, z) else {
            return;
        };
        if s.class.is_ocean {
            return;
        }
        let v = s.world.vanilla;
        let game = surface_biome(v.c, v.e, v.w, v.t, v.h);
        let ours = s.cell.biome;
        let y = s.world.height.surface_y;
        tally.land += 1;
        if game != ours && !is_river(game) && !is_river(ours) {
            tally.differ += 1;
            *tally.pairs.entry((ours, game)).or_default() += 1;
        }
        if is_mountain(game) {
            if y < limit {
                tally.mountain_low += 1;
                if is_cold_mountain(game) && s.cell.t != 0 {
                    tally.cold_on_warm_low += 1;
                    if tally.examples.len() < 8 {
                        tally.examples.push((x, z, y, ours, game));
                    }
                }
            } else {
                tally.mountain_high += 1;
            }
        }
    };
    for offset in OFFSETS {
        for k in 0..LENGTH / STEP {
            let t = (k * STEP - LENGTH / 2) as f64;
            probe(center.0 + t, center.1 + offset as f64);
            probe(center.0 + offset as f64, center.1 + t);
        }
    }
    if tally.land == 0 {
        return Err("на линиях нет суши".into());
    }
    let pct = |n: u64| 100.0 * n as f64 / tally.land as f64;
    println!(
        "Д-118 биом игры против нашего: seed={} mode={} version={} проб суши {}",
        generator.seed() as i64,
        params.mode.as_str(),
        params.version.as_str(),
        tally.land
    );
    println!(
        "  не совпадает (без рек): {} ({:.2} %)",
        tally.differ,
        pct(tally.differ)
    );
    let mut pairs: Vec<_> = tally.pairs.iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(a.1));
    for ((ours, game), n) in pairs.iter().take(12) {
        println!("    наш {ours:<24} → игра {game:<24} {n}");
    }
    println!(
        "  горные биомы игры ниже y {limit}: {} ({:.2} %), из них холодные на не снежном: {}",
        tally.mountain_low,
        pct(tally.mountain_low),
        tally.cold_on_warm_low
    );
    println!(
        "  горные биомы игры от y {limit}: {} ({:.2} %)",
        tally.mountain_high,
        pct(tally.mountain_high)
    );
    for (x, z, y, ours, game) in &tally.examples {
        println!("    пример X {x} Z {z} y {y:.0}: наш {ours}, игра {game}");
    }
    Ok(())
}

struct ReportEntry {
    ranges: [(f64, f64); 5],
    biome: String,
}

fn range(value: &Json) -> Option<(f64, f64)> {
    if let Some(v) = value.as_f64() {
        return Some((v, v));
    }
    let a = value.as_array()?;
    Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?))
}

fn dist(r: (f64, f64), v: f64) -> f64 {
    if v < r.0 {
        r.0 - v
    } else if v > r.1 {
        v - r.1
    } else {
        0.0
    }
}

/// Сверка с отчётом datagen. Точки поверхности: depth 0, offset 0.
pub fn check_report(path: &str, samples: usize) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let root = json::parse(&text)?;
    let list = root
        .get("biomes")
        .and_then(Json::as_array)
        .ok_or("в отчёте нет biomes")?;
    let mut entries = Vec::new();
    for item in list {
        let p = item.get("parameters").ok_or("нет parameters")?;
        let get = |k: &str| p.get(k).and_then(range).ok_or(format!("нет {k}"));
        let depth = get("depth")?;
        let offset = p.get("offset").and_then(Json::as_f64).unwrap_or(0.0);
        // Только поверхность: depth содержит 0 и без смещения.
        if dist(depth, 0.0) > 0.0 || offset != 0.0 {
            continue;
        }
        let biome = item
            .get("biome")
            .and_then(Json::as_str)
            .ok_or("нет biome")?
            .trim_start_matches("minecraft:")
            .to_owned();
        entries.push(ReportEntry {
            ranges: [
                get("continentalness")?,
                get("erosion")?,
                get("weirdness")?,
                get("temperature")?,
                get("humidity")?,
            ],
            biome,
        });
    }
    if entries.is_empty() {
        return Err("в отчёте нет поверхностных записей".into());
    }
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut mismatches: BTreeMap<(String, &'static str), u64> = BTreeMap::new();
    let mut examples = Vec::new();
    let mut bad = 0;
    let mut edges = 0;
    for _ in 0..samples {
        let c = -1.2 + 2.2 * next();
        let rest = [next(), next(), next(), next()].map(|u| -1.0 + 2.0 * u);
        let point = [c, rest[0], rest[1], rest[2], rest[3]];
        let mut best = f64::INFINITY;
        let mut want = "";
        let mut cell = &entries[0];
        for entry in &entries {
            let d: f64 = entry
                .ranges
                .iter()
                .zip(point)
                .map(|(r, v)| dist(*r, v).powi(2))
                .sum();
            if d < best {
                best = d;
                want = &entry.biome;
                cell = entry;
            }
        }
        // У самой границы ячейки отчёт хранит f32, копия — f64: точка в
        // тысячной от края не считается расхождением.
        let edge = cell
            .ranges
            .iter()
            .zip(point)
            .any(|(r, v)| (v - r.0).abs() < 1e-3 || (v - r.1).abs() < 1e-3);
        let got = surface_biome(point[0], point[1], point[2], point[3], point[4]);
        if got != want && edge {
            edges += 1;
        } else if got != want {
            bad += 1;
            *mismatches.entry((want.to_owned(), got)).or_default() += 1;
            if examples.len() < 40 {
                examples.push((point, want.to_owned(), got));
            }
        }
    }
    println!(
        "Д-118 сверка с отчётом {path}: записей поверхности {}, точек {samples}, расхождений {bad} (у самой границы ячейки ещё {edges}, не в счёт)",
        entries.len()
    );
    for ((want, got), n) in &mismatches {
        println!("    отчёт {want:<26} копия {got:<26} {n}");
    }
    let band = |v: f64, edges: &[f64]| edges.iter().filter(|e| v > **e).count();
    for (p, want, got) in &examples {
        let c = ["coast", "near", "mid", "far"][band(p[0], &[-0.11, 0.03, 0.3]).min(3)];
        let e = band(p[1], &[-0.78, -0.375, -0.2225, 0.05, 0.45, 0.55]);
        let w = p[2].abs();
        let slice = if w < 0.05 {
            "valley"
        } else if w < 0.266_666_68 {
            "low"
        } else if w < 0.4 {
            "mid"
        } else if w < 0.566_666_66 {
            "high"
        } else if w < 0.766_666_7 {
            "peaks"
        } else if w < 0.933_333_3 {
            "high"
        } else {
            "mid"
        };
        let t = band(p[3], &[-0.45, -0.15, 0.2, 0.55]);
        let h = band(p[4], &[-0.35, -0.1, 0.1, 0.3]);
        let sign = if p[2] < 0.0 { "−" } else { "+" };
        println!(
            "    {slice}{sign} C {c} E{e} T{t} H{h} (c={:.3} e={:.3} w={:.3} t={:.3} h={:.3}): отчёт {want}, копия {got}",
            p[0], p[1], p[2], p[3], p[4]
        );
    }
    if bad > 0 {
        return Err(format!("копия таблицы расходится с отчётом в {bad} точках"));
    }
    Ok(())
}
