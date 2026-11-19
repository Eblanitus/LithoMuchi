//! К4: доля берегов океана, где в 4 блоках от воды суша выше моря на
//! 10+ блоков (после игрового теста 2026-11-15 таких было ~20 %).
//! Берег — переход вода→суша вдоль прямой линии, вода — поверхность ниже
//! уровня моря у изолинии `c = ocean_c_threshold` (озёра и реки не считаются:
//! у них континентальность сухопутная). Горные берега (эрозия ниже
//! `height.shore_cliff_e`) считаются отдельно — обрыв там остаётся по плану.
//! Замер «до» — `--set height.shore_width_blocks=0`.

use litho_core::Generator;

/// Шаг пробы вдоль линии, блоки: критерий К4 мерит на 4 блоках от воды.
const STEP: i64 = 1;
/// Длина линии, блоки.
const LENGTH: i64 = 8192;
/// Смещения линий от центра, блоки. Линии идут по обеим осям.
const OFFSETS: [i64; 4] = [-6000, -2000, 2000, 6000];
/// Ниже этой высоты проба считается водой (уровень моря 63, запас на дробь).
const WATER_MAX_Y: f64 = 62.75;
/// Сколько блоков от воды отступаем вглубь суши для пробы К4.
const INSET: i64 = 4;
/// Порог «суша выше моря на 10+ блоков».
const OFFENSE_MIN_ABOVE_SEA: f64 = 10.0;

#[derive(Default)]
struct Tally {
    /// Берегов океана всего.
    shores: u64,
    /// Из них горных (эрозия ниже `shore_cliff_e`).
    mountain: u64,
    /// Берегов с сушей выше моря на 10+ в `INSET` блоках от воды.
    offenses: u64,
    /// Горных среди обидчиков.
    mountain_offenses: u64,
    /// Пробы суши на `INSET` блоков от воды: высота над морем.
    above_sea: Vec<f64>,
    /// Самый высокий берег: (над морем, x, z).
    worst: (f64, f64, f64),
    /// Гистограмма эрозии обидчиков: корзины по 0,25 от −1 до 1.
    offense_e: [u64; 8],
}

fn e_bucket(e: f64) -> usize {
    (((e + 1.0) * 4.0).floor().clamp(0.0, 7.0)) as usize
}

fn line(generator: &Generator, tally: &mut Tally, at: impl Fn(i64) -> (f64, f64)) {
    let threshold = generator.params().climate.ocean_c_threshold;
    let cliff_e = generator.params().height.shore_cliff_e;
    let mut in_water = false;
    let mut ocean_seen = false;
    let mut probes: Vec<Option<(f64, f64, f64)>> = Vec::with_capacity((LENGTH / STEP) as usize);
    for k in 0..LENGTH / STEP {
        let (x, z) = at(k * STEP - LENGTH / 2);
        probes.push(
            generator
                .sample(x, z)
                .map(|full| (full.world.height.surface_y, full.vanilla.c, full.vanilla.e)),
        );
    }
    for (k, probe) in probes.iter().enumerate() {
        match probe {
            None => {
                in_water = false;
                ocean_seen = false;
            }
            Some((y, c, _)) => {
                if *y < WATER_MAX_Y {
                    // Вода: запоминаем, был ли в прогоне океан (изолиния C).
                    // Реки и озёра стоят на сухопутной континентальности.
                    in_water = true;
                    if *c < threshold {
                        ocean_seen = true;
                    }
                    continue;
                }
                // Суша. Переход вода→суша даёт берег, если в воде был океан.
                if in_water && ocean_seen {
                    let metric = k + (INSET / STEP) as usize;
                    if let Some((y_metric, _, e_metric)) = probes.get(metric).copied().flatten() {
                        if y_metric >= WATER_MAX_Y {
                            let above = y_metric - litho_core::height::SEA_LEVEL;
                            let mountain = e_metric < cliff_e;
                            tally.shores += 1;
                            tally.mountain += u64::from(mountain);
                            let offense = above >= OFFENSE_MIN_ABOVE_SEA;
                            tally.offenses += u64::from(offense);
                            tally.mountain_offenses += u64::from(offense && mountain);
                            if !mountain {
                                tally.above_sea.push(above);
                            }
                            if above > tally.worst.0 {
                                tally.worst = (above, x_of(&at, k), z_of(&at, k));
                            }
                            if offense {
                                tally.offense_e[e_bucket(e_metric)] += 1;
                            }
                        }
                    }
                }
                in_water = false;
                ocean_seen = false;
            }
        }
    }
}

fn x_of(at: &impl Fn(i64) -> (f64, f64), k: usize) -> f64 {
    at(k as i64 * STEP - LENGTH / 2).0
}

fn z_of(at: &impl Fn(i64) -> (f64, f64), k: usize) -> f64 {
    at(k as i64 * STEP - LENGTH / 2).1
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
    if tally.shores == 0 {
        return Err("на линиях нет ни одного берега океана".into());
    }
    let regular = tally.shores - tally.mountain;
    let regular_offenses = tally.offenses - tally.mountain_offenses;
    let pct = |part: u64, whole: u64| {
        if whole == 0 {
            0.0
        } else {
            100.0 * part as f64 / whole as f64
        }
    };
    let mean = if tally.above_sea.is_empty() {
        0.0
    } else {
        tally.above_sea.iter().sum::<f64>() / tally.above_sea.len() as f64
    };
    println!(
        "shores: {n} линий по {LENGTH} блоков, шаг {STEP}; берегов океана {shores} (горных {mountain})",
        n = 2 * OFFSETS.len(),
        shores = tally.shores,
        mountain = tally.mountain
    );
    println!(
        "  в {INSET} блоках от воды выше моря на {OFFENSE_MIN_ABOVE_SEA:.0}+: {} из {regular} берегов ({:.1} %) — цель К4: единицы процентов",
        regular_offenses,
        pct(regular_offenses, regular)
    );
    println!(
        "  горные берега: {} обидчиков из {} ({:.1} %) — вне гейта",
        tally.mountain_offenses,
        tally.mountain,
        pct(tally.mountain_offenses, tally.mountain)
    );
    println!("  обычный берег: высота в +4 над морем в среднем {mean:.1} блоков");
    println!(
        "  худший берег: +{:.1} над морем в ({}, {})",
        tally.worst.0, tally.worst.1, tally.worst.2
    );
    let bands = [
        "e −1,00…−0,75",
        "−0,75…−0,50",
        "−0,50…−0,25",
        "−0,25…0,00",
        "0,00…0,25",
        "0,25…0,50",
        "0,50…0,75",
        "0,75…1,00",
    ];
    let buckets: String = bands
        .iter()
        .zip(tally.offense_e)
        .filter(|(_, count)| *count > 0)
        .map(|(name, count)| format!("    {name}: {count}"))
        .collect::<Vec<_>>()
        .join("\n");
    if !buckets.is_empty() {
        println!("  эрозия обидчиков:");
        println!("{buckets}");
    }
    Ok(())
}
