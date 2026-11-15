//! К4а/часть 2: счётчик крутых ступеней на прямых линиях с разбивкой по
//! причинам (Д-106).
//!
//! Ступень — перепад больше 3 блоков на блок между соседними пробами шагом
//! 2 блока. Считаются только пары, где обе точки — суша выше моря: океан и
//! берег линии разрывают. Категории:
//! - вырез воды (`height.water_strength`): край русла/котловины; из них
//!   «стол со стеной» — вырезанная точка выше суши рядом (баг К3);
//! - у реки (`river` > 0) — борта долины, PV → −1;
//! - шов коры: `mountain`/`plateau` прыгают между пробами;
//! - горы: эрозия ниже `height.shore_cliff_e` (E0/E1, обрывы задуманы);
//! - таблица E/PV: ступень от движения полей по крутой таблице высот
//!   (разложение через таблицу: чей вклад больше — E или PV), с подсчётом
//!   доли внутри полосы затухания у низины (`relief.land_fade_km()`).

use litho_core::Generator;
use litho_core::height::{self, SEA_LEVEL};

/// Шаг пробы вдоль линии, блоки.
const STEP: i64 = 2;
/// Длина линии, блоки.
const LENGTH: i64 = 16_384;
/// Смещения линий от центра, блоки: те же, что у `biomes` (К1, Д-103).
const OFFSETS: [i64; 4] = [-6000, -2000, 2000, 6000];
/// Порог ступени: перепад больше 3 блоков на блок.
const STEEP_PER_BLOCK: f64 = 3.0;
/// Доля вклада, с которой ступень приписывается одному полю.
const MAIN_SHARE: f64 = 0.6;

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
    /// Поле эрозии, вход таблицы высот.
    e: f64,
    /// Собственный PV, вход таблицы высот (уже с речным PV → −1).
    pv: f64,
    c: f64,
    e_km: f64,
    /// К4: расстояние до изолинии океана, блоки (диагностика рампы).
    shore: f64,
    /// Наложки гидрологии на E/C (диагностика прыжков полей).
    floodplain: f64,
    estuary: f64,
    inland_sea: f64,
    mountain: f64,
    plateau: f64,
}

impl Point {
    fn near_water(self) -> bool {
        self.cut > 0.0 || self.deferred > 0.0 || self.lake > 0.0
    }
}

/// Чистое значение таблицы высот на суше: без воды, бугров и берега.
fn table_y(generator: &Generator, e: f64, pv: f64, e_km: f64) -> f64 {
    height::sample_micro(
        1.0,
        e,
        pv,
        e_km,
        0.0,
        0.0,
        0.0,
        0.0,
        f64::INFINITY,
        generator.params(),
    )
    .surface_y
}

fn point(generator: &Generator, x: f64, z: f64) -> Option<Point> {
    let sample = generator.sample(x, z)?;
    let height = &sample.world.height;
    let hydrology = &sample.world.hydrology;
    let surface = &sample.world.climate.surface;
    Some(Point {
        y: height.surface_y,
        land: !sample.class.is_ocean && height.surface_y > SEA_LEVEL,
        cut: height.water_strength,
        deferred: hydrology.r2_deferred,
        lake: hydrology.lakes.water,
        river: sample.world.river,
        e: sample.world.vanilla.e,
        pv: sample.world.pv,
        c: sample.world.vanilla.c,
        e_km: surface.e_km,
        shore: generator.shore_distance_blocks(x, z, sample.world.vanilla.c),
        floodplain: hydrology.floodplain,
        estuary: hydrology.estuary,
        inland_sea: hydrology.lakes.inland_sea,
        mountain: surface.mountain,
        plateau: surface.plateau,
    })
}

/// Категории сухопутных ступеней.
#[derive(Default)]
struct DryTally {
    mountain: u64,
    crust_seam: u64,
    erosion: u64,
    pv: u64,
    mixed: u64,
    /// Из табличных: нижняя точка внутри полосы затухания у низины.
    fade_strip: u64,
}

#[derive(Default)]
struct Tally {
    pairs: u64,
    steep: u64,
    /// Крутые края выреза воды: одна точка вырезана, вторая — нет.
    water_edge: u64,
    /// Из них «стол со стеной»: вырезанная точка выше суши рядом.
    raised: u64,
    river: u64,
    dry: DryTally,
    /// Самая крутая не-горная ступень: перепад, x, z, категория.
    worst_dry: (f64, f64, f64, &'static str),
    /// До пяти худших не-горных ступеней: (перепад, x, z, категория,
    /// y_low, e_low, pv_low, c_low, e_km_low, табличное y_low,
    /// e_high, pv_high, mountain_hi, plateau_hi, табличное y_high,
    /// shore_low, shore_high, floodplain_lo, floodplain_hi, estuary_hi,
    /// inland_sea_hi).
    top: Vec<(
        f64,
        f64,
        f64,
        &'static str,
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
    )>,
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
                    let cut_a = a.cut > 0.0;
                    let cut_b = b.cut > 0.0;
                    if cut_a != cut_b {
                        tally.water_edge += 1;
                        // Подъём считаем по вырезанной точке: она должна быть
                        // ниже суши рядом (К3).
                        let (cut, land) = if cut_a { (a.y, b.y) } else { (b.y, a.y) };
                        if cut - land > 1.0 {
                            tally.raised += 1;
                        }
                    } else if a.river > 0.0 || b.river > 0.0 {
                        tally.river += 1;
                    } else if !a.near_water() && !b.near_water() {
                        classify_dry(generator, tally, &a, &b, bx, bz, dy);
                    }
                }
            }
        }
        previous = Some((b, bx, bz));
    }
}

/// Разложение сухопутной ступени по причинам (Д-106, часть 2).
fn classify_dry(
    generator: &Generator,
    tally: &mut Tally,
    a: &Point,
    b: &Point,
    bx: f64,
    bz: f64,
    dy: f64,
) {
    let category = if a.e.min(b.e) < generator.params().height.shore_cliff_e {
        "горы"
    } else if (a.mountain - b.mountain).abs() > 0.5 || (a.plateau - b.plateau).abs() > 0.5 {
        "шов коры"
    } else {
        // Разложение по таблице: чьё поле двинуло высоту сильнее.
        let (lo, hi) = if a.y <= b.y { (a, b) } else { (b, a) };
        let base = table_y(generator, lo.e, lo.pv, lo.e_km);
        let dy_e = table_y(generator, hi.e, lo.pv, lo.e_km) - base;
        let dy_pv = table_y(generator, lo.e, hi.pv, lo.e_km) - base;
        let (main, share) = if dy_e.abs() >= dy_pv.abs() {
            ("E", dy_e.abs() / (dy_e.abs() + dy_pv.abs()).max(1e-9))
        } else {
            ("PV", dy_pv.abs() / (dy_e.abs() + dy_pv.abs()).max(1e-9))
        };
        if lo.e_km < generator.params().relief.land_fade_km() {
            tally.dry.fade_strip += 1;
        }
        if share >= MAIN_SHARE {
            if main == "E" {
                tally.dry.erosion += 1;
            } else {
                tally.dry.pv += 1;
            }
            main
        } else {
            tally.dry.mixed += 1;
            "E+PV"
        }
    };
    if category != "горы" && dy > tally.worst_dry.0 {
        tally.worst_dry = (dy, bx, bz, category);
    }
    if category != "горы" {
        let (lo, hi) = if a.y <= b.y { (a, b) } else { (b, a) };
        let table_lo = table_y(generator, lo.e, lo.pv, lo.e_km);
        let table_hi = table_y(generator, hi.e, hi.pv, hi.e_km);
        tally.top.push((
            dy,
            bx,
            bz,
            category,
            lo.y,
            lo.e,
            lo.pv,
            lo.c,
            lo.e_km,
            table_lo,
            hi.e,
            hi.pv,
            hi.mountain,
            hi.plateau,
            table_hi,
            lo.shore,
            hi.shore,
            lo.floodplain,
            hi.floodplain,
            hi.estuary,
            hi.inland_sea,
        ));
        tally.top.sort_by(|x, y| y.0.total_cmp(&x.0));
        tally.top.truncate(5);
    }
}

#[allow(clippy::too_many_lines)]
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
    let inland = tally.dry.crust_seam + tally.dry.erosion + tally.dry.pv + tally.dry.mixed;
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
        "  у выреза воды: {} (из них «стол со стеной»: {})",
        tally.water_edge, tally.raised
    );
    println!("  у реки: {}", tally.river);
    println!(
        "  суша вдали от воды: {} — горы {}, шов коры {}, таблица E {}, PV {}, смешанные {} (из них в полосе затухания: {})",
        tally.dry.mountain
            + tally.dry.crust_seam
            + tally.dry.erosion
            + tally.dry.pv
            + tally.dry.mixed,
        tally.dry.mountain,
        tally.dry.crust_seam,
        tally.dry.erosion,
        tally.dry.pv,
        tally.dry.mixed,
        tally.dry.fade_strip
    );
    println!(
        "  суша вне гор: {inland} ступеней; самая крутая {:.0} блоков на {STEP} у ({:.0}, {:.0}), категория {}",
        tally.worst_dry.0, tally.worst_dry.1, tally.worst_dry.2, tally.worst_dry.3
    );
    for step in &tally.top {
        println!(
            "    {:.0} бл. на {STEP} у ({:.0}, {:.0}), {}: низ y {:.0} (табл. {:.0}) e {:+.3} pv {:+.2} c {:+.2} e_km {:.3} берег {:.0} пойма {:.2}; высок e {:+.3} pv {:+.2} (табл. {:.0}) mnt {:.2} plat {:.2} берег {:.0} пойма {:.2} устье {:.2} внморе {:.2}",
            step.0,
            step.1,
            step.2,
            step.3,
            step.4,
            step.9,
            step.5,
            step.6,
            step.7,
            step.8,
            step.15,
            step.16,
            step.10,
            step.11,
            step.14,
            step.12,
            step.13,
            step.17,
            step.18,
            step.19,
            step.20
        );
    }
    Ok(())
}

/// Максимальный уклон поверхности вокруг точки по сетке шагом 4 блока
/// в квадрате со стороной `2 * radius`, блоков на блок (проверка К4а).
pub fn slope_around(generator: &Generator, center: (f64, f64), radius: f64) -> f64 {
    let step = 4.0;
    let mut worst = 0.0_f64;
    let base = generator
        .sample(center.0, center.1)
        .map(|s| s.world.height.surface_y);
    let Some(base_y) = base else {
        return 0.0;
    };
    let n = (radius / step) as i64;
    for dxi in -n..=n {
        for dzi in -n..=n {
            if dxi == 0 && dzi == 0 {
                continue;
            }
            let (dx, dz) = (f64::from(dxi as i32), f64::from(dzi as i32));
            let x = center.0 + dx * step;
            let z = center.1 + dz * step;
            if let Some(sample) = generator.sample(x, z) {
                let y = sample.world.height.surface_y;
                let dist = ((dx * step).powi(2) + (dz * step).powi(2)).sqrt().max(step);
                worst = worst.max((y - base_y).abs() / dist);
            }
        }
    }
    worst
}
