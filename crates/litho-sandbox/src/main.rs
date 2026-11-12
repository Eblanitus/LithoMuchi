//! Песочница Litospherity: карты, метрики, атлас и пробы (план 12.2).

mod acceptance;
mod height;
mod height_stats;
mod json;
mod palette;
mod png;
mod render;
mod tune;

use std::path::Path;
use std::time::Instant;

use height::{HeightModel, HeightSource};
use litho_core::channel::{ReachEnd, ReachKind};
use litho_core::drainage::{DepressionKind, DrainageOutlet};
use litho_core::lake::LakeKind;
use litho_core::{Field, GenVersion, Generator, Mode, Params};
use render::{LAYERS, RenderOptions};

/// 16 сидов для `sweep` (12.5, Э1h): и отрицательные, и большие значения.
const SWEEP_SEEDS: [i64; 16] = [
    0,
    1,
    2,
    7,
    42,
    123,
    777,
    1234,
    2024,
    12_345,
    54_321,
    99_999,
    -1,
    -42,
    -123_456_789,
    987_654_321,
];

fn value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1).cloned())
}

fn parse_pair(text: &str) -> Result<(f64, f64), String> {
    let mut parts = text.split(',');
    let a = parts.next().unwrap_or("").trim();
    let b = parts
        .next()
        .ok_or("нужны две координаты через запятую")?
        .trim();
    let x = a
        .parse::<f64>()
        .map_err(|e| format!("координата '{a}': {e}"))?;
    let z = b
        .parse::<f64>()
        .map_err(|e| format!("координата '{b}': {e}"))?;
    Ok((x, z))
}

fn options_from(args: &[String]) -> Result<RenderOptions, String> {
    let seed = match value(args, "--seed") {
        Some(text) => text.parse::<i64>().map_err(|e| format!("--seed: {e}"))?,
        None => 42,
    };
    let mode = match value(args, "--mode") {
        Some(text) => Mode::parse(&text).ok_or_else(|| format!("неизвестный режим '{text}'"))?,
        None => Mode::Gameplay,
    };
    let version = match value(args, "--version") {
        Some(text) => {
            GenVersion::parse(&text).ok_or_else(|| format!("неизвестная версия '{text}'"))?
        }
        None => GenVersion::V0,
    };
    let (center_x, center_z) = match value(args, "--center") {
        Some(text) => parse_pair(&text)?,
        None => (0.0, 0.0),
    };
    let size = match value(args, "--size") {
        Some(text) => text.parse::<usize>().map_err(|e| format!("--size: {e}"))?,
        None => 512,
    };
    let bpp = match value(args, "--bpp") {
        Some(text) => text.parse::<f64>().map_err(|e| format!("--bpp: {e}"))?,
        None => match mode {
            Mode::Scientific => 64.0,
            Mode::Gameplay => 16.0,
        },
    };
    let path_fast = match value(args, "--path") {
        Some(text) if text == "reference" => false,
        Some(text) if text == "fast" => true,
        Some(text) => {
            return Err(format!(
                "--path: ожидалось reference|fast, получено '{text}'"
            ));
        }
        None => false,
    };
    if size == 0 || !bpp.is_finite() || bpp <= 0.0 {
        return Err("--size должен быть > 0, --bpp строго > 0".to_string());
    }
    Ok(RenderOptions {
        seed,
        mode,
        version,
        center_x,
        center_z,
        size,
        bpp,
        path_fast,
        pv: args.iter().any(|arg| arg == "--pv"),
    })
}

fn generator(opts: &RenderOptions) -> Result<Generator, String> {
    tune::generator(opts.seed, opts.mode, opts.version)
}

fn height_source(args: &[String]) -> Result<Option<HeightSource>, String> {
    let data = value(args, "--game-data");
    let synthetic = args.iter().any(|arg| arg == "--synthetic-height");
    match (data, synthetic) {
        (Some(_), true) => Err("--game-data и --synthetic-height взаимоисключающи".into()),
        (Some(path), false) => Ok(Some(HeightSource::GameData(path.into()))),
        (None, true) => Ok(Some(HeightSource::Synthetic)),
        (None, false) => Ok(None),
    }
}

/// T144: v1-dev по умолчанию показывает собственную высоту ядра; явные
/// --game-data/--synthetic-height остаются явным переопределением предпросмотра.
fn height_model_for(args: &[String], opts: &RenderOptions) -> Result<Option<HeightModel>, String> {
    if let Some(source) = height_source(args)? {
        return Ok(Some(HeightModel::load(source)?));
    }
    if opts.version.is_water_relief_dev() {
        return Ok(Some(HeightModel::load(HeightSource::Owned)?));
    }
    Ok(None)
}

fn cmd_render(args: &[String]) -> Result<(), String> {
    let opts = options_from(args)?;
    let layers = match value(args, "--layers") {
        Some(text) if text == "all" => LAYERS
            .iter()
            .filter(|layer| {
                !matches!(**layer, "channels" | "lakes" | "hydrology" | "relief")
                    || opts.version.is_water_relief_dev()
            })
            .map(|layer| (*layer).to_string())
            .collect(),
        Some(text) => text.split(',').map(|s| s.trim().to_string()).collect(),
        None => vec!["plates".to_string()],
    };
    let out = value(args, "--out").unwrap_or_else(|| "out".to_string());
    let dir = Path::new(&out);
    std::fs::create_dir_all(dir).map_err(|e| format!("не могу создать '{out}': {e}"))?;
    let params_hash = Params::defaults(opts.version, opts.mode).params_hash();
    let height = height_model_for(args, &opts)?;
    if layers.iter().any(|layer| layer == "vanilla-height") && height.is_none() {
        return Err(
            "слой vanilla-height для v0 требует --game-data ПУТЬ или явный --synthetic-height; v1-dev использует собственную высоту автоматически".into(),
        );
    }
    for layer in layers {
        let image = render::render_layer_with_height(&opts, &layer, height.as_ref())?;
        let png_path = dir.join(format!("{layer}.png"));
        png::write_png(&png_path, &image).map_err(|e| {
            let path = png_path.display();
            format!("запись '{path}': {e}")
        })?;
        let json_path = dir.join(format!("{layer}.json"));
        let meta_opts = render::options_for_layer(&opts, &layer);
        std::fs::write(
            &json_path,
            render::metadata_with_height(&meta_opts, &layer, params_hash, height.as_ref()),
        )
        .map_err(|e| {
            let path = json_path.display();
            format!("запись '{path}': {e}")
        })?;
        println!("{layer}: {}", png_path.display());
    }
    Ok(())
}

fn cmd_probe(args: &[String]) -> Result<(), String> {
    let opts = options_from(args)?;
    let at = value(args, "--at").ok_or("нужно --at x,z")?;
    let (x, z) = parse_pair(&at)?;
    println!("{}", render::probe_text(&opts, x, z)?);
    Ok(())
}

/// 10 точек вокруг точки появления для сверки с игрой (14.5, п. 2).
fn cmd_checkpoints(args: &[String]) -> Result<(), String> {
    let opts = options_from(args)?;
    let generator = generator(&opts)?;
    let shift = generator.spawn_shift();
    let l = opts.mode.l_blocks();
    println!(
        "p* = ({px}, {pz}) L = ({bx}, {bz}) блоков, params={hash:016x}",
        px = shift.x,
        pz = shift.z,
        bx = shift.x * l,
        bz = shift.z * l,
        hash = generator.params().params_hash(),
    );
    // Направления фиксированы таблицей: системная тригонометрия не нужна.
    let dirs = [
        (1.0_f64, 0.0_f64),
        (0.809, 0.588),
        (0.309, 0.951),
        (-0.309, 0.951),
        (-0.809, 0.588),
        (-1.0, 0.0),
        (-0.809, -0.588),
        (-0.309, -0.951),
        (0.309, -0.951),
        (0.809, -0.588),
    ];
    for (k, (dx, dz)) in dirs.iter().enumerate() {
        let radius = 200.0 + 150.0 * k as f64;
        let x = shift.x * l + dx * radius;
        let z = shift.z * l + dz * radius;
        println!("--- точка {k}: ({x:.0}, {z:.0}) ---");
        println!("{}", render::probe_text(&opts, x, z)?);
    }
    Ok(())
}

/// Скачки C/E/W между соседними блоками: команда `seams` (12.3, 12.5 В1).
/// Порог 14.2: соседние блоки не должны расходиться больше чем на 0.02.
/// Не ослабляется: это тот же порог, что и в `adjacent_blocks_are_continuous`.
const SEAM_LIMIT: f64 = 0.02;

/// Отношение «быстрый путь / эталон» на main 8d44a17: 989.3 / 917.1 = 1.079.
/// Допуск 2 % на шум разделяемого раннера; абсолютный бюджет 14.4 (150 мкс)
/// пока не выполнен и честно печатается отдельно (Д-019, пункт 5 в работе).
const BENCH_MAX_FAST_OVER_REF: f64 = 1.10;
/// К4 / шаг 6: замер на чанк 16×16 колонок. Цель по умолчанию — один кадр
/// 60 FPS; она предварительная и не согласована автором (tools/bench-targets.json).
const DEFAULT_CHUNK_TARGET_MS: f64 = 16.0;
const CHUNK_SIDE: usize = 16;
const CHUNK_REPEATS: usize = 3;

/// Разложение полей в худшей точке: видно, какой именно физический вклад
/// сделал скачок. Печатается всегда, в том числе когда `seams` пусто.
fn seam_components(generator: &Generator, x: f64, z: f64, label: &str) {
    let Some(crust) = generator.crust_sample(x, z) else {
        return;
    };
    let (d_ab, d_ac, w_b, w_c, nx, nz) = match generator.plate_sample(x, z) {
        Some(p) => (p.d_ab_l, p.d_ac_l, p.w_b, p.w_c, p.n_ab.x, p.n_ab.z),
        None => (f64::NAN, f64::NAN, 0.0, 0.0, f64::NAN, f64::NAN),
    };
    let c = litho_core::vanilla::continentalness(crust.e_km, crust.iota_l, generator.params());
    let k = crust.boundary_weights;
    println!(
        "  {label} ({x:.0},{z:.0}): C={c:.5} e={e:.5} κ={kap:.4} χ={chi:.3} ι={iota:.4} M={mnt:.3} Pl={pl:.3}",
        e = crust.e_km,
        kap = crust.kappa_l,
        chi = crust.chi,
        iota = crust.iota_l,
        mnt = crust.mountain,
        pl = crust.plateau,
    );
    println!(
        "    e = base {b:+.5} + границы {u:+.5} + точки {h:+.5} + деталь {d:+.5} км; \
         d_ab={dab:.4} d_ac={dac:.4} w_b={wb:.3} w_c={wc:.3} n=({nx:+.2},{nz:+.2})",
        b = crust.e_base_km,
        u = crust.boundary_u_km,
        h = crust.hotspot_km,
        d = crust.detail_km,
        dab = d_ab,
        dac = d_ac,
        wb = w_b,
        wc = w_c,
        nx = nx,
        nz = nz,
    );
    println!(
        "    плиты A/B/C = {p0}/{p1}/{p2}; χ_plate = [{c0:.2}, {c1:.2}, {c2:.2}]; \
         upper_is_a={up} sigma_b={sig:+.4}",
        p0 = crust.plate_ids[0],
        p1 = crust.plate_ids[1],
        p2 = crust.plate_ids[2],
        c0 = crust.chi_plates[0],
        c1 = crust.chi_plates[1],
        c2 = crust.chi_plates[2],
        up = crust.upper_is_a,
        sig = crust.sigma_b_l,
    );
    println!(
        "    k1..k7 = [{:.3}, {:.3}, {:.3}, {:.3}, {:.3}, {:.3}, {:.3}]",
        k[0], k[1], k[2], k[3], k[4], k[5], k[6]
    );
}

/// Полный скан карты, а не 100 точек юнит-теста: 64×64 узла с шагом 16
/// блоков, соседи по обеим осям для всех трёх полей. Возвращает ошибку при
/// любом превышении порога — это и есть обязательный гейт из Д-022.
/// Диагностика Д-059: `--ray-lattice ШАГ_L` подменяет шаг решётки луча
/// влаги быстрого пути (0 — луч из каждой колонки, как в эталоне).
fn with_ray_lattice(generator: Generator, args: &[String]) -> Result<Generator, String> {
    let Some(text) = value(args, "--ray-lattice") else {
        return Ok(generator);
    };
    let step = text
        .parse::<f64>()
        .map_err(|e| format!("--ray-lattice: {e}"))?;
    if !(0.0..=1.0).contains(&step) {
        return Err("--ray-lattice: шаг должен лежать в 0…1 L".to_string());
    }
    Ok(generator.with_ray_lattice(step))
}

/// Д-059: сверка быстрого пути с эталоном на сетке точек. Быстрый путь
/// v1-dev берёт луч влаги с решётки; здесь видно, сколько это стоит в μ,
/// осадках, полях T/H, высоте и классе зоны. `--min-zone-share X` делает
/// команду гейтом.
fn cmd_paths(args: &[String]) -> Result<(), String> {
    let opts = options_from(args)?;
    let generator = with_ray_lattice(generator(&opts)?, args)?;
    let n = match value(args, "--n") {
        Some(text) => text.parse::<usize>().map_err(|e| format!("--n: {e}"))?,
        None => 48,
    };
    let span_l = match value(args, "--span-l") {
        Some(text) => text.parse::<f64>().map_err(|e| format!("--span-l: {e}"))?,
        None => 40.0,
    };
    let min_share = value(args, "--min-zone-share")
        .map(|text| {
            text.parse::<f64>()
                .map_err(|e| format!("--min-zone-share: {e}"))
        })
        .transpose()?;
    if n == 0 || !span_l.is_finite() || span_l <= 0.0 {
        return Err("--n должен быть > 0, --span-l строго > 0".to_string());
    }
    let step = span_l * opts.mode.l_blocks() / n as f64;
    let names = ["μ", "P, мм", "T", "H", "C", "E", "W", "высота, блоков"];
    let mut errors: Vec<Vec<f64>> = vec![Vec::with_capacity(n * n); names.len()];
    let mut worst = [(0.0_f64, 0.0_f64, 0.0_f64); 8];
    let (mut total, mut same, mut land, mut land_same) = (0_usize, 0_usize, 0_usize, 0_usize);
    for j in 0..n {
        for i in 0..n {
            // Полблока сдвига: точки не попадают в узлы решётки луча.
            let x = (opts.center_x + (i as f64 - n as f64 * 0.5) * step).floor() + 0.5;
            let z = (opts.center_z + (j as f64 - n as f64 * 0.5) * step).floor() + 0.5;
            let (Some(reference), Some(fast)) = (
                generator
                    .try_sample_reference(x, z)
                    .map_err(|e| e.to_string())?,
                generator.try_sample(x, z).map_err(|e| e.to_string())?,
            ) else {
                continue;
            };
            total += 1;
            let zone_same = reference.zone == fast.zone;
            same += usize::from(zone_same);
            if !reference.world.is_ocean {
                land += 1;
                land_same += usize::from(zone_same);
            }
            let deltas = [
                reference.climate.precip.mu - fast.climate.precip.mu,
                reference.climate.precip.p_mm - fast.climate.precip.p_mm,
                reference.vanilla.t - fast.vanilla.t,
                reference.vanilla.h - fast.vanilla.h,
                reference.vanilla.c - fast.vanilla.c,
                reference.vanilla.e - fast.vanilla.e,
                reference.vanilla.w - fast.vanilla.w,
                reference.world.height.surface_y - fast.world.height.surface_y,
            ];
            for (k, delta) in deltas.iter().enumerate() {
                let delta = delta.abs();
                errors[k].push(delta);
                if delta > worst[k].0 {
                    worst[k] = (delta, x, z);
                }
            }
        }
    }
    if total == 0 {
        return Err("paths: в окне нет ни одной точки с плитой".to_string());
    }
    let share = same as f64 / total as f64;
    let land_share = if land == 0 {
        1.0
    } else {
        land_same as f64 / land as f64
    };
    println!(
        "paths ({version}, {mode}, сид {seed}): {total} точек, окно {span_l} L, шаг {step:.0} блоков",
        version = opts.version.as_str(),
        mode = opts.mode.as_str(),
        seed = opts.seed,
    );
    println!(
        "paths: зона совпала в {share:.4} точек; на суше {land_share:.4} ({land_same}/{land})"
    );
    let nodes = generator.ray_nodes();
    println!(
        "paths: узлов луча влаги {nodes} — {per:.4} на колонку (эталон: 1 луч на колонку)",
        per = nodes as f64 / total as f64
    );
    for (k, name) in names.iter().enumerate() {
        let list = &mut errors[k];
        list.sort_by(f64::total_cmp);
        let mean = list.iter().sum::<f64>() / list.len() as f64;
        let p95 = list[(list.len() - 1) * 95 / 100];
        let (max, x, z) = worst[k];
        println!("paths: |Δ {name}| среднее {mean:.5}, p95 {p95:.5}, max {max:.5} в ({x}, {z})");
    }
    match min_share {
        Some(min) if share < min => Err(format!(
            "paths: совпадение зон {share:.4} ниже требуемого {min}"
        )),
        _ => Ok(()),
    }
}

fn cmd_seams(args: &[String]) -> Result<(), String> {
    let opts = options_from(args)?;
    let generator = generator(&opts)?;
    let n = 64_usize;
    let step = 16.0;
    let mut worst_c = (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
    let mut worst_e = worst_c;
    let mut worst_w = worst_c;
    for i in 0..n {
        for j in 0..n {
            let x = (i as f64 - n as f64 * 0.5) * step;
            let z = (j as f64 - n as f64 * 0.5) * step;
            let base = [
                generator.field(Field::Continents, x, 64.0, z),
                generator.field(Field::Erosion, x, 64.0, z),
                generator.field(Field::Ridges, x, 64.0, z),
            ];
            for (dx, dz) in [(1.0, 0.0), (0.0, 1.0)] {
                let next = [
                    generator.field(Field::Continents, x + dx, 64.0, z + dz),
                    generator.field(Field::Erosion, x + dx, 64.0, z + dz),
                    generator.field(Field::Ridges, x + dx, 64.0, z + dz),
                ];
                for (worst, k) in [(&mut worst_c, 0), (&mut worst_e, 1), (&mut worst_w, 2)] {
                    let delta = (next[k] - base[k]).abs();
                    if delta > worst.0 {
                        *worst = (delta, x, z, base[k], next[k]);
                    }
                }
            }
        }
    }
    let (max_c, cx, cz, ch, cn) = worst_c;
    let (max_e, ex, ez, eh, en) = worst_e;
    let (max_w, wx, wz, wh, wn) = worst_w;
    println!(
        "seams: скан {scan}×{scan} узлов, шаг {step:.0} блоков, обе оси; \
         max |ΔC|={max_c:.5} |ΔE|={max_e:.5} |ΔW|={max_w:.5}, порог {lim}",
        scan = n,
        lim = SEAM_LIMIT,
    );
    println!("  C: ({cx:.0},{cz:.0}) {ch:.5} → {cn:.5}");
    println!("  E: ({ex:.0},{ez:.0}) {eh:.5} → {en:.5}");
    println!("  W: ({wx:.0},{wz:.0}) {wh:.5} → {wn:.5}");
    seam_components(&generator, cx, cz, "C");
    seam_components(&generator, cx + 1.0, cz, "C+1x");
    seam_components(&generator, cx, cz + 1.0, "C+1z");
    seam_components(&generator, ex, ez, "E");
    seam_components(&generator, wx, wz, "W");
    if max_c <= SEAM_LIMIT && max_e <= SEAM_LIMIT && max_w <= SEAM_LIMIT {
        println!("seams: пусто");
        return Ok(());
    }
    println!("seams: ЕСТЬ РАЗРЫВЫ");
    Err(format!(
        "seams: |ΔC|={max_c:.5}, |ΔE|={max_e:.5}, |ΔW|={max_w:.5} \
         при пороге {SEAM_LIMIT}; мерж запрещён"
    ))
}

/// Метрики 12.4 по области 200×200 L (T63).
/// Метрики 12.4 плюс диагностика климата на суше (Д-023).
#[derive(Clone, Copy)]
struct StatsRow {
    seed: i64,
    mode: Mode,
    land: f64,
    mountains: f64,
    deserts: f64,
    forests: f64,
    polar: f64,
    p_mm: f64,
    pet_mm: f64,
    ratio: f64,
    mu: f64,
    cont: f64,
    t_bio: f64,
}

fn stats_row(seed: i64, mode: Mode) -> Result<StatsRow, String> {
    let generator = Generator::new(seed, mode, GenVersion::V0).map_err(|e| e.to_string())?;
    let params = generator.params();
    let n = 64_usize;
    let step = 200.0 * params.block.l_blocks / n as f64;
    let mut land = 0_usize;
    let mut mountains = 0_usize;
    let mut deserts = 0_usize;
    let mut forests = 0_usize;
    let mut polar = 0_usize;
    let (mut p_sum, mut pet_sum, mut ratio_sum) = (0.0_f64, 0.0_f64, 0.0_f64);
    let (mut mu_sum, mut cont_sum, mut t_sum) = (0.0_f64, 0.0_f64, 0.0_f64);
    for i in 0..n {
        for j in 0..n {
            let x = (i as f64 - n as f64 * 0.5) * step;
            let z = (j as f64 - n as f64 * 0.5) * step;
            let Some(world) = generator.world_sample(x, z) else {
                continue;
            };
            if !world.is_ocean {
                land += 1;
                if world.climate.surface.mountain > 0.5 {
                    mountains += 1;
                }
                let zone = world.class.zone;
                if zone.province <= 2 {
                    deserts += 1;
                }
                if zone.province >= 5 && (2..=6).contains(&zone.belt) {
                    forests += 1;
                }
                if zone.belt <= 1 {
                    polar += 1;
                }
                let pr = world.climate.precip;
                p_sum += pr.p_mm;
                pet_sum += pr.pet_mm;
                ratio_sum += pr.ratio;
                mu_sum += pr.mu;
                cont_sum += pr.cont;
                t_sum += world.climate.temperature.t_bio;
            }
        }
    }
    let total = (n * n).max(1) as f64;
    let land_n = land.max(1) as f64;
    Ok(StatsRow {
        seed,
        mode,
        land: 100.0 * land as f64 / total,
        mountains: 100.0 * mountains as f64 / land_n,
        deserts: 100.0 * deserts as f64 / land_n,
        forests: 100.0 * forests as f64 / land_n,
        polar: 100.0 * polar as f64 / land_n,
        p_mm: p_sum / land_n,
        pet_mm: pet_sum / land_n,
        ratio: ratio_sum / land_n,
        mu: mu_sum / land_n,
        cont: cont_sum / land_n,
        t_bio: t_sum / land_n,
    })
}

fn cmd_stats(args: &[String]) -> Result<(), String> {
    let opts = options_from(args)?;
    if let Some(model) = height_model_for(args, &opts)? {
        let stats_opts = height_stats::StatsOptions::parse(args, &opts)?;
        let stats = height_stats::HeightStats::collect(&opts, &stats_opts, &model)?;
        if args.iter().any(|arg| arg == "--json") {
            print!("{}", stats.json(&opts, &stats_opts, &model));
        } else {
            print!("{}", stats.text(&opts, &stats_opts, &model));
        }
        return Ok(());
    }
    let row = stats_row(opts.seed, opts.mode)?;
    println!(
        "сид={seed} режим={mode} область=200×200 L точек=4096\n\
         доля суши              {land:6.1} %  (цель 25–40)\n\
         горы на суше           {mnt:6.1} %  (цель 5–20)\n\
         пустыни на суше        {dst:6.1} %  (цель 15–30)\n\
         леса на суше           {frs:6.1} %  (цель 30–50)\n\
         полярные пояса на суше {pol:6.1} %  (цель 10–25)",
        seed = row.seed,
        mode = row.mode.as_str(),
        land = row.land,
        mnt = row.mountains,
        dst = row.deserts,
        frs = row.forests,
        pol = row.polar,
    );
    // Д-023: честная диагностика, почему пустынь много, а лесов мало.
    println!(
        "климат на суше (среднее): осадки P={p:7.1} мм, испаряемость PET={pet:7.1} мм, PET/P={r:5.2}, влага μ={mu:5.3}, континентальность c={cont:5.3}, t_bio={t:5.1} °C",
        p = row.p_mm,
        pet = row.pet_mm,
        r = row.ratio,
        mu = row.mu,
        cont = row.cont,
        t = row.t_bio,
    );
    Ok(())
}

/// Оценка 12.5 / Э1h: те же метрики на 16 сидах × 2 режима, без картинок.
fn cmd_stats_all(args: &[String]) -> Result<(), String> {
    if let Some(model) = height_model_for(args, &options_from(args)?)? {
        let base = options_from(args)?;
        let opts = height_stats::StatsOptions::parse(args, &base)?;
        let json_output = args.iter().any(|arg| arg == "--json");
        let mut rows = Vec::new();
        if !json_output {
            println!("T100: 16 сидов × 2 режима; {}", model.source.label());
            if model.is_synthetic() {
                println!("WARNING: синтетический сплайн, НЕ ваниль; приёмка требует --game-data.");
            }
            println!("{}", height_stats::COMPACT_HEADER);
        }
        for seed in SWEEP_SEEDS {
            for mode in Mode::ALL {
                let mut run = base.clone();
                run.seed = seed;
                run.mode = mode;
                let stats = height_stats::HeightStats::collect(&run, &opts, &model)?;
                if json_output {
                    rows.push(stats.json(&run, &opts, &model));
                } else {
                    println!("{}", stats.compact());
                }
            }
        }
        if json_output {
            println!(
                "[{}]",
                rows.iter()
                    .map(|row| row.trim())
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
        return Ok(());
    }
    println!(
        "сид        режим      суша  горы пустыни  леса поляр |     P    PET PET/P     μ     c  t_bio"
    );
    let mut n = 0_usize;
    let mut sum = StatsRow {
        seed: 0,
        mode: Mode::Gameplay,
        land: 0.0,
        mountains: 0.0,
        deserts: 0.0,
        forests: 0.0,
        polar: 0.0,
        p_mm: 0.0,
        pet_mm: 0.0,
        ratio: 0.0,
        mu: 0.0,
        cont: 0.0,
        t_bio: 0.0,
    };
    for seed in SWEEP_SEEDS {
        for mode in Mode::ALL {
            let r = stats_row(seed, mode)?;
            println!(
                "{seed:>10} {mode:<10} {land:5.1} {mnt:5.1} {dst:8.1} {frs:5.1} {pol:5.1} | {p:7.1} {pet:7.1} {r:5.2} {mu:5.3} {cont:5.3} {t:6.1}",
                seed = r.seed,
                mode = r.mode.as_str(),
                land = r.land,
                mnt = r.mountains,
                dst = r.deserts,
                frs = r.forests,
                pol = r.polar,
                p = r.p_mm,
                pet = r.pet_mm,
                r = r.ratio,
                mu = r.mu,
                cont = r.cont,
                t = r.t_bio,
            );
            n += 1;
            sum.land += r.land;
            sum.mountains += r.mountains;
            sum.deserts += r.deserts;
            sum.forests += r.forests;
            sum.polar += r.polar;
            sum.p_mm += r.p_mm;
            sum.pet_mm += r.pet_mm;
            sum.ratio += r.ratio;
            sum.mu += r.mu;
            sum.cont += r.cont;
            sum.t_bio += r.t_bio;
        }
    }
    let d = n as f64;
    println!(
        "СРЕДНЕЕ ({n} прогонов): суша {land:5.1} горы {mnt:5.1} пустыни {dst:5.1} леса {frs:5.1} поляр {pol:5.1} | P {p:7.1} PET {pet:7.1} PET/P {r:5.2} μ {mu:5.3} c {cont:5.3} t_bio {t:5.1}",
        land = sum.land / d,
        mnt = sum.mountains / d,
        dst = sum.deserts / d,
        frs = sum.forests / d,
        pol = sum.polar / d,
        p = sum.p_mm / d,
        pet = sum.pet_mm / d,
        r = sum.ratio / d,
        mu = sum.mu / d,
        cont = sum.cont / d,
        t = sum.t_bio / d,
    );
    Ok(())
}

/// T110–T115: диагностика сети стока, русел, озёр и дельт плюс эффективная
/// поверхность R1. Собственного PV и воды выше моря пока нет.
fn cmd_drainage(args: &[String]) -> Result<(), String> {
    let opts = options_from(args)?;
    let (x, z) = parse_pair(&value(args, "--at").unwrap_or_else(|| "0,0".to_string()))?;
    if !x.is_finite() || !z.is_finite() {
        return Err("--at требует конечных координат".into());
    }
    let generator = generator(&opts)?;
    let plate = generator
        .plate_sample(x, z)
        .ok_or("в этой точке не найдена плита")?
        .a;
    let start = Instant::now();
    let net = generator
        .drainage_network(&plate)
        .map_err(|error| error.to_string())?;
    let cold_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let _warm = generator
        .drainage_network(&plate)
        .map_err(|error| error.to_string())?;
    let warm_ms = start.elapsed().as_secs_f64() * 1000.0;
    net.check_invariants(generator.params().drainage.fill_epsilon_km)
        .map_err(|error| error.to_string())?;
    let nodes = net.nodes();
    let plate_nodes = net.plate_node_count();
    let sea = nodes
        .iter()
        .filter(|n| n.outlet == DrainageOutlet::Sea)
        .count();
    let rift = nodes
        .iter()
        .filter(|n| n.outlet == DrainageOutlet::Rift)
        .count();
    let transform = nodes
        .iter()
        .filter(|n| n.outlet == DrainageOutlet::Transform)
        .count();
    let filled = net
        .depressions()
        .iter()
        .filter(|d| matches!(d.kind, DepressionKind::Filled { .. }))
        .count();
    let closed = net.depressions().len() - filled;
    let max_depth = net
        .depressions()
        .iter()
        .map(|d| d.max_depth_km)
        .fold(0.0, f64::max);
    let runoff = net.runoff();
    let climate_nodes = runoff.climate_nodes();
    let endorheic = runoff.endorheic_count();
    let export = runoff.export_mm_l2();
    let evaporation = runoff.evaporation_mm_l2();
    let storage = runoff.storage_mm_l2();
    let max_q = runoff
        .nodes()
        .iter()
        .map(|node| node.discharge_mm_l2)
        .fold(0.0, f64::max);
    let channels = net.channels();
    let reaches = channels.reaches().len();
    let channel_segments = channels.segments().len();
    let baskets = channels.basket_count();
    let channel_length = channels.total_length_l();
    let unresolved = channels.unresolved_count();
    let channel_boundary = channels.end_count(ReachEnd::Boundary);
    let channel_below_threshold = channels.end_count(ReachEnd::BelowThreshold);
    let channel_closed_surplus = channels.end_count(ReachEnd::ClosedSurplus);
    let delta_count = channels.delta_count();
    let delta_arms = channels.delta_arm_count();
    let lakes = net.lakes();
    let lake_count = lakes.lakes().len();
    let lake_water = lakes.kind_count(LakeKind::Lake);
    let lake_inland_sea = lakes.kind_count(LakeKind::InlandSea);
    let lake_swamp = lakes.kind_count(LakeKind::Swamp);
    let lake_dry = lakes.kind_count(LakeKind::DryBasin);
    let lake_closed = lakes.kind_count(LakeKind::ClosedSurplus);
    let lake_sea_connected = lakes.kind_count(LakeKind::SeaConnected);
    let lake_frozen = lakes.lakes().iter().filter(|lake| lake.frozen).count();
    let lake_rift = lakes.lakes().iter().filter(|lake| lake.rift).count();
    let lake_high = lakes.lakes().iter().filter(|lake| lake.high).count();
    let lake_water_area = lakes.water_area_l2();
    let lake_unresolved = lakes.unresolved_count();
    let mut plain_sinuosity: Vec<f64> = channels
        .reaches()
        .iter()
        .filter(|reach| reach.kind == ReachKind::Channel && reach.plain && reach.sinuosity > 0.0)
        .map(|reach| reach.sinuosity)
        .collect();
    plain_sinuosity.sort_by(f64::total_cmp);
    let median_sinuosity = if plain_sinuosity.is_empty() {
        0.0
    } else {
        let mid = plain_sinuosity.len() / 2;
        if plain_sinuosity.len() % 2 == 0 {
            0.5 * (plain_sinuosity[mid - 1] + plain_sinuosity[mid])
        } else {
            plain_sinuosity[mid]
        }
    };
    let query_start = Instant::now();
    let (channel_query, lake_query) = generator
        .drainage_surface_query(x, z)
        .map_err(|error| error.to_string())?;
    let query_ms = query_start.elapsed().as_secs_f64() * 1000.0;
    let effective = generator
        .hydrology_at(x, z)
        .map_err(|error| error.to_string())?;
    let effective_kind = effective.kind.as_str();
    let effective_river = effective.river;
    let effective_water = effective.water;
    let effective_deferred = effective.r2_deferred;
    let effective_swamp = effective.swamp;
    let effective_dry = effective.dry_basin;
    let effective_floodplain = effective.floodplain;
    let effective_gallery = effective.gallery;
    let query_radius = litho_core::channel::max_query_radius_l(generator.params());
    let query_networks = channel_query.networks_considered;
    let query_core = channel_query.core;
    let query_valley = channel_query.valley;
    let query_plate = channel_query
        .nearest
        .map_or_else(|| "null".to_string(), |hit| hit.plate_id.to_string());
    let query_distance = channel_query.nearest.map_or_else(
        || "null".to_string(),
        |hit| format!("{:.9}", hit.distance_l),
    );
    let query_delta = channel_query.delta;
    let lake_query_water = lake_query.water;
    let lake_query_shore = lake_query.shore;
    let lake_query_basin = lake_query.basin;
    let lake_query_kind = lake_query
        .nearest
        .map_or_else(|| "null".to_string(), |hit| hit.kind.as_str().to_string());
    let lake_query_level = lake_query
        .nearest
        .and_then(|hit| hit.level_km)
        .map_or_else(|| "null".to_string(), |level| format!("{level:.9}"));
    let hash = generator.params().params_hash();
    if args.iter().any(|arg| arg == "--json") {
        println!(
            "{{\"stage\":\"T110-T116\",\"acceptance\":false,\"connected_to_game_fields\":true,\"effective_kind\":\"{effective_kind}\",\"effective_river\":{effective_river:.9},\"effective_water\":{effective_water:.9},\"effective_deferred\":{effective_deferred:.9},\"effective_swamp\":{effective_swamp:.9},\"effective_dry\":{effective_dry:.9},\"effective_floodplain\":{effective_floodplain:.9},\"effective_gallery\":{effective_gallery:.9},\"seed\":{},\"mode\":\"{}\",\"version\":\"{}\",\"params_hash\":\"{hash:016x}\",\"plate_id\":{},\"at\":[{x},{z}],\"nodes_total\":{},\"nodes_plate\":{},\"nodes_ring\":{},\"outlets_sea\":{sea},\"outlets_rift\":{rift},\"outlets_transform\":{transform},\"depressions_filled\":{filled},\"depressions_closed\":{closed},\"max_depth_km\":{max_depth:.9},\"climate_nodes\":{climate_nodes},\"endorheic_basins\":{endorheic},\"max_discharge_mm_l2\":{max_q:.9},\"export_mm_l2\":{export:.9},\"evaporation_mm_l2\":{evaporation:.9},\"storage_mm_l2\":{storage:.9},\"channel_reaches\":{reaches},\"channel_segments\":{channel_segments},\"channel_baskets\":{baskets},\"channel_length_l\":{channel_length:.9},\"channel_unresolved\":{unresolved},\"channel_boundary\":{channel_boundary},\"channel_below_threshold\":{channel_below_threshold},\"channel_closed_surplus\":{channel_closed_surplus},\"plain_sinuosity_median\":{median_sinuosity:.9},\"channel_query_networks\":{query_networks},\"channel_query_radius_l\":{query_radius:.9},\"channel_query_core\":{query_core:.9},\"channel_query_valley\":{query_valley:.9},\"channel_query_plate\":{query_plate},\"channel_query_distance_l\":{query_distance},\"channel_query_delta\":{query_delta:.9},\"delta_fans\":{delta_count},\"delta_arms\":{delta_arms},\"lake_surfaces\":{lake_count},\"lake_water\":{lake_water},\"lake_inland_sea\":{lake_inland_sea},\"lake_swamp\":{lake_swamp},\"lake_dry_basin\":{lake_dry},\"lake_closed_surplus\":{lake_closed},\"lake_sea_connected\":{lake_sea_connected},\"lake_frozen\":{lake_frozen},\"lake_rift\":{lake_rift},\"lake_high_r2\":{lake_high},\"lake_water_area_l2\":{lake_water_area:.9},\"lake_unresolved\":{lake_unresolved},\"lake_query_water\":{lake_query_water:.9},\"lake_query_shore\":{lake_query_shore:.9},\"lake_query_basin\":{lake_query_basin:.9},\"lake_query_kind\":\"{lake_query_kind}\",\"lake_query_level_km\":{lake_query_level},\"channel_query_ms\":{query_ms:.6},\"cold_ms\":{cold_ms:.6},\"warm_ms\":{warm_ms:.6}}}",
            opts.seed,
            opts.mode.as_str(),
            opts.version.as_str(),
            plate.id,
            nodes.len(),
            plate_nodes,
            nodes.len() - plate_nodes,
        );
    } else {
        println!(
            "T110–T116 drainage: сеть подключена к C/E/W и биомам (R1); собственного PV/воды выше моря нет\n\
             seed={seed} mode={mode} version={version} params={hash:016x} plate={plate}\n\
             узлы: всего {total}, своей плиты {plate_nodes}, кольцо {ring}; выходы: море {sea}, рифт {rift}, трансформ {transform}\n\
             впадины: заполненные {filled}, замкнутые без выхода {closed}, бессточные {endorheic}; max глубина {max_depth:.6} км\n\
             климат: {climate_nodes} узлов; max Q {max_q:.3} мм·L²/год; экспорт {export:.3}, испарение {evaporation:.3}, замкнутый избыток {storage:.3}\n\
             русла T112: {reaches} цепочек, {channel_segments} отрезков, {baskets} корзин, длина {channel_length:.3} L; незакрытых концов {unresolved} (граница плиты {channel_boundary}, ниже Q_min {channel_below_threshold}, замкнутый избыток {channel_closed_surplus}); медиана извилистости равнинных {median_sinuosity:.3}\n\
             дельты T114: вееров {delta_count}, рукавов {delta_arms}; озёра/котловины: поверхностей {lake_count} (озеро {lake_water}, внутр. море {lake_inland_sea}, болото {lake_swamp}, сухая {lake_dry}, замкнутый избыток {lake_closed}, морская связь {lake_sea_connected}), замёрзших {lake_frozen}, рифтовых {lake_rift}, высоких R2 {lake_high}, площадь воды {lake_water_area:.3} L², без уровня {lake_unresolved}\n\
             общий запрос T113/T114: сетей {query_networks}, радиус {query_radius:.3} L, ядро {query_core:.3}, долина {query_valley:.3}, дельта {query_delta:.3}; ближайшее русло плиты {query_plate}, расстояние {query_distance} L; озеро вода {lake_query_water:.3}, берег {lake_query_shore:.3}, котловина {lake_query_basin:.3}, тип {lake_query_kind}, уровень {lake_query_level} км; запрос {query_ms:.3} мс\n\
             T115 эффективно: тип {effective_kind}, река {effective_river:.3}, вода R1 {effective_water:.3}, отложено R2 {effective_deferred:.3}, галерейный лес {effective_gallery:.3}\n\
             время: cold {cold_ms:.3} мс, warm {warm_ms:.3} мс (один раннер, не приёмка T160)",
            seed = opts.seed,
            mode = opts.mode.as_str(),
            version = opts.version.as_str(),
            plate = plate.id,
            total = nodes.len(),
            ring = nodes.len() - plate_nodes,
        );
    }
    Ok(())
}

/// T79: цена редких колонок дальнего рельефа (LOD). Horizon просит тайлы
/// 16 × 16 колонок с шагом 4…128 блоков и для каждой колонки зовёт
/// `field_column`. Здесь тот же рисунок запросов: квадрат соседних тайлов у
/// центра, все холодные (решётки луча сброшены, сети рек прогреты).
fn cmd_lod(args: &[String]) -> Result<(), String> {
    let opts = options_from(args)?;
    let tiles = match value(args, "--tiles") {
        Some(text) => text.parse::<usize>().map_err(|e| format!("--tiles: {e}"))?,
        None => 3,
    };
    let strides: Vec<i64> = match value(args, "--strides") {
        Some(text) => text
            .split(',')
            .map(|part| {
                part.trim()
                    .parse::<i64>()
                    .map_err(|e| format!("--strides: {e}"))
            })
            .collect::<Result<_, _>>()?,
        None => vec![4, 8, 16, 32, 64, 128],
    };
    if tiles == 0 || strides.is_empty() || strides.iter().any(|s| *s <= 0) {
        return Err("--tiles должен быть > 0, шаги --strides — целые > 0".to_string());
    }
    let json = args.iter().any(|arg| arg == "--json");
    let fields = [
        Field::Continents,
        Field::Erosion,
        Field::Ridges,
        Field::Temperature,
        Field::Vegetation,
    ];
    const SIDE: i64 = 16;
    // Точек прогрева сетей рек на сторону тайла.
    const WARM: i64 = 9;
    let mut generator = with_ray_lattice(generator(&opts)?, args)?;
    let mut sink = 0.0_f64;
    let mut rows = Vec::new();
    for &stride in &strides {
        // Тайлы стоят на сетке своего шага вплотную друг к другу квадратом
        // `tiles × tiles`, как их просит Horizon: соседние тайлы делят узлы
        // решёток, но не колонки.
        let span = stride * SIDE;
        let (cx, cz) = (
            (opts.center_x / span as f64).floor() as i64 * span,
            (opts.center_z / span as f64).floor() as i64 * span,
        );
        let origins: Vec<(i64, i64)> = (0..tiles * tiles)
            .map(|k| {
                let (i, j) = ((k % tiles) as i64, (k / tiles) as i64);
                (cx + i * span, cz + j * span)
            })
            .collect();
        // Сети рек строятся один раз на плиту и меряются отдельно
        // (`drainage`): прогреваем их в точках вне сетки колонок.
        for &(ox, oz) in &origins {
            for wz in 0..WARM {
                for wx in 0..WARM {
                    let fx = wx as f64 / (WARM - 1) as f64;
                    let fz = wz as f64 / (WARM - 1) as f64;
                    sink += generator.field_column(
                        Field::Continents,
                        ox as f64 + fx * (span - stride) as f64 + 0.5,
                        oz as f64 + fz * (span - stride) as f64 + 0.5,
                    );
                }
            }
        }
        let lattice_l = generator.params().boundaries.ray_lattice_l;
        generator = generator.with_ray_lattice(lattice_l);
        let mut per_tile = Vec::with_capacity(origins.len());
        for &(ox, oz) in &origins {
            let start = Instant::now();
            for iz in 0..SIDE {
                for ix in 0..SIDE {
                    let x = (ox + ix * stride) as f64;
                    let z = (oz + iz * stride) as f64;
                    for field in fields {
                        sink += generator.field_column(field, x, z);
                    }
                }
            }
            per_tile.push(start.elapsed().as_secs_f64() * 1e3 / (SIDE * SIDE) as f64);
        }
        per_tile.sort_by(f64::total_cmp);
        let median = per_tile[per_tile.len() / 2];
        let worst = per_tile[per_tile.len() - 1];
        let columns = (origins.len() as i64 * SIDE * SIDE) as f64;
        let mean = per_tile.iter().sum::<f64>() / per_tile.len() as f64;
        let rays = generator.ray_nodes() as f64 / columns;
        let nodes = generator.ray_field_nodes() as f64 / columns;
        rows.push((stride, mean, median, worst, rays, nodes));
    }
    if json {
        let body = rows
            .iter()
            .map(|(stride, mean, median, worst, rays, nodes)| {
                format!(
                    "{{\"stride\":{stride},\"ms_per_column_mean\":{mean:.5},\"ms_per_column\":{median:.5},\"ms_per_column_worst\":{worst:.5},\"rays_per_column\":{rays:.4},\"field_nodes_per_column\":{nodes:.3}}}"
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        println!(
            "{{\"seed\":{seed},\"mode\":\"{mode}\",\"version\":\"{version}\",\"tiles\":{tiles},\"ray_lattice_l\":{lattice:?},\"ray_field_l\":{field:?},\"strides\":[{body}]}}",
            seed = opts.seed,
            mode = opts.mode.as_str(),
            version = opts.version.as_str(),
            lattice = generator.params().boundaries.ray_lattice_l,
            field = generator.params().boundaries.ray_field_l,
        );
        return Ok(());
    }
    println!(
        "lod ({version}, {mode}, сид {seed}): квадрат {tiles} × {tiles} холодных тайлов по 16 × 16 колонок на шаг, решётка луча {lattice} L, решётка высот {field} L (sink={sink:.3})",
        version = opts.version.as_str(),
        mode = opts.mode.as_str(),
        seed = opts.seed,
        lattice = generator.params().boundaries.ray_lattice_l,
        field = generator.params().boundaries.ray_field_l,
    );
    println!(
        "  шаг, блоков | мс/колонку: среднее | медиана тайлов | худший тайл | лучей/колонку | узлов высот/колонку"
    );
    for (stride, mean, median, worst, rays, nodes) in rows {
        println!(
            "  {stride:>11} | {mean:>19.4} | {median:>14.4} | {worst:>11.4} | {rays:>13.3} | {nodes:>19.2}"
        );
    }
    Ok(())
}

fn cmd_bench(args: &[String]) -> Result<(), String> {
    let opts = options_from(args)?;
    let chunk_target_ms = match value(args, "--chunk-target-ms") {
        Some(text) => text
            .parse::<f64>()
            .map_err(|e| format!("--chunk-target-ms: {e}"))?,
        None => DEFAULT_CHUNK_TARGET_MS,
    };
    if !chunk_target_ms.is_finite() || chunk_target_ms <= 0.0 {
        return Err("--chunk-target-ms должен быть конечным и > 0".into());
    }
    let enforce_chunk_target = args.iter().any(|arg| arg == "--enforce-chunk-target");
    let generator = with_ray_lattice(generator(&opts)?, args)?;
    let n = 1000;
    let mut sink = 0.0_f64;
    let mut refs = [0.0_f64; 3];
    let mut fasts = [0.0_f64; 3];
    for (r, f) in refs.iter_mut().zip(fasts.iter_mut()) {
        // Медиана трёх сглаживает шум разделяемого раннера, чтобы гейт не
        // срабатывал от чужой нагрузки на CPU. Отношение двух путей считается
        // в одном процессе и потому не зависит от абсолютной скорости машины.
        let start = Instant::now();
        for k in 0..n {
            let x = k as f64 * 0.75;
            if let Some(full) = generator.sample_reference(x, -x * 0.5) {
                sink += full.vanilla.c;
            }
        }
        *r = start.elapsed().as_secs_f64() / n as f64 * 1e6;
        let start = Instant::now();
        for k in 0..n {
            sink += generator.field(Field::Continents, k as f64 * 0.75, 64.0, -k as f64 * 0.375);
        }
        *f = start.elapsed().as_secs_f64() / n as f64 * 1e6;
    }
    refs.sort_by(f64::total_cmp);
    fasts.sort_by(f64::total_cmp);
    let reference = refs[1];
    let fast = fasts[1];
    let ratio = reference / fast.max(1e-9);
    println!(
        "bench: медиана 3 прогонов — эталон {reference:.1} мкс/точку, быстрый путь {fast:.1} мкс/поле, ускорение {ratio:.3}× (sink={sink:.3})"
    );
    println!(
        "bench: бюджет 14.4 = 150 мкс/поле — {budget}; гейт регрессии: быстрый/эталон ≤ {limit:.2} (прошлое 1.079, допуск 2 %)",
        budget = if fast <= 150.0 {
            "выполнен"
        } else {
            "НЕ выполнен (Д-019, пункт 5 в работе)"
        },
        limit = BENCH_MAX_FAST_OVER_REF,
    );
    // T160: кэш колонки — несколько полей в одной (x,z). Это диагностика,
    // не новый гейт: cold считает первое поле колонки, warm — попадание кэша.
    let column_fields = [
        Field::Continents,
        Field::Erosion,
        Field::Ridges,
        Field::Temperature,
        Field::Vegetation,
    ];
    let cn = 256_usize;
    let mut column_pass = || {
        let start = Instant::now();
        for k in 0..cn {
            let x = k as f64 * 0.75;
            for field in column_fields {
                sink += generator.field(field, x, 64.0, -x * 0.375);
            }
        }
        start.elapsed().as_secs_f64() / (cn * column_fields.len()) as f64 * 1e6
    };
    let column_cold = column_pass();
    let column_warm = column_pass();
    println!(
        "bench T160: колонка 5 полей — cold {column_cold:.1} мкс/поле, warm {column_warm:.1} мкс/поле, кэш {ratio:.2}× (sink={sink:.3})",
        ratio = column_cold / column_warm.max(1e-9)
    );
    // К4 / шаг 6: отдельный замер на чанк и на колонку для v1-dev. Повторы
    // показывают и холодный проход, и попадание в кэш колонки T160.
    let chunk_columns = CHUNK_SIDE * CHUNK_SIDE;
    // Д-057: сети рек соседних плит строятся один раз на плиту (сотни мс) и
    // меряются отдельно (`drainage`, cold). Здесь нужна цена колонок, поэтому
    // углы и центр области трогаем заранее, в точках вне сетки чанка: иначе
    // одна постройка сети, случайно попавшая в чанк, утраивает «мс/чанк».
    let chunk_span = ((CHUNK_SIDE - 1) * 4) as f64;
    let (chunk_x, chunk_z) = (opts.center_x.floor(), opts.center_z.floor());
    for (fx, fz) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (0.5, 0.5)] {
        sink += generator.field(
            Field::Continents,
            chunk_x + fx * chunk_span + 0.5,
            64.0,
            chunk_z + fz * chunk_span + 0.5,
        );
    }
    // Д-059: прогрев и замеры выше уже посчитали узлы решётки луча влаги
    // вокруг чанка. Сбрасываем решётку и кэш колонок (сети рек остаются):
    // холодный чанк обязан заплатить за свои лучи сам.
    let lattice_l = generator.params().boundaries.ray_lattice_l;
    let generator = generator.with_ray_lattice(lattice_l);
    // Д-057: холодный проход — ровно один. Раньше время трёх повторов (один
    // холодный и два из кэша) делилось на три, и «cold» выходил втрое меньше.
    let mut chunk_pass = |repeats: usize| {
        let start = Instant::now();
        for _ in 0..repeats {
            for iz in 0..CHUNK_SIDE {
                for ix in 0..CHUNK_SIDE {
                    let x = chunk_x + (ix * 4) as f64;
                    let z = chunk_z + (iz * 4) as f64;
                    for field in column_fields {
                        sink += generator.field(field, x, 64.0, z);
                    }
                }
            }
        }
        start.elapsed().as_secs_f64() / repeats as f64 * 1e3
    };
    let chunk_cold = chunk_pass(1);
    let chunk_warm = chunk_pass(CHUNK_REPEATS);
    let per_column_ms = chunk_cold / chunk_columns as f64;
    if generator.params().version.is_water_relief_dev() {
        println!(
            "bench K4: лучей влаги на холодный чанк — {rays} на {chunk_columns} колонок \
             (эталон: луч на колонку); шаг решётки {lattice_l} L",
            rays = generator.ray_nodes(),
        );
    }
    let target_status = if chunk_cold <= chunk_target_ms {
        "в цели"
    } else {
        "ВЫШЕ цели"
    };
    println!(
        "bench K4 ({version}): чанк {side}×{side}×{fields} — cold {cold:.2} мс/чанк, \
         warm {warm:.2} мс/чанк, {column:.3} мс/колонка, цель {target:.2} мс/чанк — \
         {status} ({gate}; sink={sink:.3})",
        version = opts.version.as_str(),
        side = CHUNK_SIDE,
        fields = column_fields.len(),
        cold = chunk_cold,
        warm = chunk_warm,
        column = per_column_ms,
        target = chunk_target_ms,
        status = target_status,
        gate = if enforce_chunk_target {
            "гейт включён"
        } else {
            "диагностика, гейт выключен"
        },
        sink = sink
    );
    if enforce_chunk_target && chunk_cold > chunk_target_ms {
        return Err(format!(
            "bench: cold чанк {chunk_cold:.2} мс > цели {chunk_target_ms:.2} мс"
        ));
    }
    if fast > reference * BENCH_MAX_FAST_OVER_REF {
        return Err(format!(
            "bench: регрессия — быстрый путь {fast:.1} мкс против эталона {reference:.1} мкс \
             (в {slow:.3} раза), прошлое было 1.079; мерж запрещён",
            slow = fast / reference.max(1e-9),
        ));
    }
    Ok(())
}

fn atlas_html() -> String {
    let mut cards = String::new();
    for seed in SWEEP_SEEDS {
        for mode in Mode::ALL {
            let name = format!("s{seed}-{mode}", mode = mode.as_str());
            cards.push_str(&format!(
                "  <div class=\"card\"><a href=\"sweep/{name}.png\"><img src=\"sweep/{name}.png\" alt=\"{name}\"></a>\n    <p>Сид {seed} · {label}</p></div>\n",
                label = if mode == Mode::Scientific { "научный режим" } else { "геймплейный режим" },
            ));
        }
    }
    format!(
        r#"<!doctype html>
<html lang="ru"><head><meta charset="utf-8"><title>Litospherity — атлас 16 сидов</title>
<style>
 body {{ background:#101820; color:#e8eef4; font:16px/1.5 system-ui,sans-serif; margin:0; padding:28px; }}
 h1 {{ font-size:26px; }} p.lead {{ color:#a9bac9; max-width:900px; }}
 .grid {{ display:grid; grid-template-columns:repeat(auto-fill,minmax(230px,1fr)); gap:16px; }}
 .card {{ background:#1b2733; border:1px solid #2c3e50; border-radius:10px; padding:10px; }}
 .card img {{ width:100%; display:block; border-radius:6px; }}
 .card p {{ margin:8px 2px 2px; font-size:14px; }}
 h2 {{ margin-top:34px; font-size:20px; }}
 li {{ margin:5px 0; }}
</style></head><body>
<h1>Атлас: 16 сидов × 2 режима</h1>
<p class="lead">Каждая картинка — один и тот же слой рельефа `height` и один
и тот же кадр 25×25 км (шаг ≈97,7 блока на пиксель) в обоих режимах. Но `L` в
научном режиме вчетверо крупнее, поэтому игровой кадр — это 100×100 L, а научный —
25×25 L: одни и те же материки выглядят вчетверо крупнее, и пары больше не
совпадают до последнего пикселя. Кадр уменьшен с 50 до 25 км, а размер вырос со
192 до 256 пикселей — шаг стал в 2,7 раза мельче, материки не мельчат. Зелёное —
суша, синее — океан. Нажми, чтобы открыть в полный размер.
Это материал для оценки по чеклисту 12.5: смотри, достаточно ли разные материки,
нет ли сетки, не один ли сплошной суперматерик и не россыпь ли мелких островов.</p>
<div class="grid">
{cards}</div>
<h2>Что отметить автору (чеклист 12.5)</h2>
<ul>
<li>А1. Горы на сближающихся границах, жёлоба и островные дуги, срединные хребты и рифты.</li>
<li>А2. Пояса идут от полюсов по порядку, между полюсами — широкие экваториальные зоны.</li>
<li>А3. Пустыни на западных берегах 15–30° и в глубине материков, влажные наветренные берега.</li>
<li>А4. Тайга в глубине материков высоких широт, тундра ближе к полюсам.</li>
<li>А5. Нигде не видно сетки: прямых линий, одинаковых повторов, кругов.</li>
<li>Б1. От точки появления в пределах 2000 блоков не меньше 4 разных биомов.</li>
<li>Б3. Реки есть и большинство доходит до моря или большой реки.</li>
<li>Б4. Материки разные: не одни мелкие острова и не один сплошной суперматерик.</li>
</ul>
<p class="lead">Ответ можно дать одной фразой: «нравится / не нравится / хочу вот так».
Код читать не нужно.</p>
<p class="lead">См. также <a href="index.html" style="color:#7fc4ff">все слои сида 42</a> ·
<a href="compare.html" style="color:#7fc4ff">сравнение с обычным Minecraft</a> ·
<a href="before-after.html" style="color:#7fc4ff">что изменилось: было / стало</a>.</p>
</body></html>
"#
    )
}

fn write_atlas(dir: &Path) -> Result<(), String> {
    std::fs::write(
        dir.parent().unwrap_or(Path::new(".")).join("atlas.html"),
        atlas_html(),
    )
    .map_err(|e| format!("запись atlas.html: {e}"))
}

/// Д-024 / Э2: разбивка времени по слоям, чтобы ускорять то, что реально
/// дорого, не меняя ни одного числа и ни одного замороженного хеша.
/// Д-026 / `freeze v0`: хеши параметров прямо из кода, а не из документа.
/// CI сверяет их с зафиксированными значениями, поэтому любая правка чисел
/// краснеет ещё до пересъёмки карт.
fn cmd_hash() -> Result<(), String> {
    for mode in Mode::ALL {
        let hash = Params::defaults(GenVersion::V0, mode).params_hash();
        println!("{} = {hash:016x}", mode.as_str());
    }
    Ok(())
}

fn cmd_profile() -> Result<(), String> {
    let generator =
        Generator::new(42, Mode::Gameplay, GenVersion::V0).map_err(|error| error.to_string())?;
    let n = 2000_usize;
    let points: Vec<(f64, f64)> = (0..n)
        .map(|k| {
            let x = k as f64 * 0.75;
            (x, -x * 0.5)
        })
        .collect();

    fn measure<F>(label: &str, points: &[(f64, f64)], mut f: F) -> f64
    where
        F: FnMut(f64, f64) -> f64,
    {
        let start = Instant::now();
        let mut sink = 0.0_f64;
        for &(x, z) in points {
            sink += f(x, z);
        }
        let us = start.elapsed().as_secs_f64() / points.len() as f64 * 1e6;
        println!("profile: {label:<22} {us:8.1} мкс/точку");
        sink
    }

    let mut sink = 0.0_f64;
    sink += measure("плиты", &points, |x, z| {
        generator
            .plate_sample(x, z)
            .map(|p| p.d_ab_l)
            .unwrap_or(0.0)
    });
    sink += measure("плиты + кора", &points, |x, z| {
        generator.crust_sample(x, z).map(|c| c.e_km).unwrap_or(0.0)
    });
    sink += measure("климат (луч 32)", &points, |x, z| {
        generator
            .climate_sample(x, z)
            .map(|c| c.precip.p_mm)
            .unwrap_or(0.0)
    });
    sink += measure("эталон целиком", &points, |x, z| {
        generator
            .world_sample(x, z)
            .map(|w| w.vanilla.c)
            .unwrap_or(0.0)
    });
    sink += measure("33 полюса", &points, |x, z| {
        let base = generator.macro_point(x, z);
        let mut acc = 0.0_f64;
        for k in 0..33 {
            let q = litho_core::math::Vec2::new(base.x - k as f64 * 0.25, base.z);
            acc += litho_core::climate::pole_sample(q, 42, generator.params())
                .map(|p| p.phi_deg)
                .unwrap_or(0.0);
        }
        acc
    });
    // Д-027: где именно внутри `surface_at` уходят 850 мкс.
    sink += measure("33 kappa_global", &points, |x, z| {
        let base = generator.macro_point(x, z);
        let mut acc = 0.0_f64;
        for k in 0..33 {
            let q = litho_core::math::Vec2::new(base.x - k as f64 * 0.25, base.z);
            acc += litho_core::crust::kappa_global(q, 42, generator.params());
        }
        acc
    });
    sink += measure("33 plate_query", &points, |x, z| {
        let base = generator.macro_point(x, z);
        let mut acc = 0.0_f64;
        for k in 0..33 {
            let q = litho_core::math::Vec2::new(base.x - k as f64 * 0.25, base.z);
            acc += litho_core::plates::plate_query(q, 42, generator.params())
                .map(|p| p.d_ab_l)
                .unwrap_or(0.0);
        }
        acc
    });
    sink += measure("33 crust_from_sample", &points, |x, z| {
        let base = generator.macro_point(x, z);
        let mut acc = 0.0_f64;
        for k in 0..33 {
            let q = litho_core::math::Vec2::new(base.x - k as f64 * 0.25, base.z);
            if let Some(pl) = litho_core::plates::plate_query(q, 42, generator.params()) {
                acc += litho_core::crust::crust_from_sample(q, &pl, 42, generator.params()).e_km;
            }
        }
        acc
    });
    println!("profile: sink={sink:.3} (чтобы оптимизатор не выкинул замеры)");
    std::hint::black_box(sink);
    Ok(())
}

/// `sweep`: 16 сидов × 2 режима, слой `height`, плюс HTML-атлас (T64).
fn cmd_sweep(args: &[String]) -> Result<(), String> {
    let out = value(args, "--out").unwrap_or_else(|| "docs/images/sweep".to_string());
    let dir = Path::new(&out);
    std::fs::create_dir_all(dir).map_err(|e| format!("не могу создать '{out}': {e}"))?;
    // Д-021: раньше bpp зависел от режима и обе картинки снимали одну и ту же
    // площадь 200×200 L — пары были попиксельно одинаковыми. Теперь шаг в
    // блоках общий, а кадр уменьшен вдвое, чтобы материки не мельчали.
    let size = 256_usize;
    let bpp = 100.0 * 250.0 / size as f64;
    for seed in SWEEP_SEEDS {
        for mode in Mode::ALL {
            let opts = RenderOptions {
                seed,
                mode,
                version: GenVersion::V0,
                center_x: 0.0,
                center_z: 0.0,
                size,
                bpp,
                path_fast: false,
                pv: false,
            };
            let image = render::render_layer(&opts, "height")?;
            let name = format!("s{seed}-{mode}", mode = mode.as_str());
            let png_path = dir.join(format!("{name}.png"));
            png::write_png(&png_path, &image)
                .map_err(|e| format!("запись '{}': {e}", png_path.display()))?;
            let hash = Params::defaults(GenVersion::V0, mode).params_hash();
            std::fs::write(
                dir.join(format!("{name}.json")),
                render::metadata(&opts, "height", hash),
            )
            .map_err(|e| format!("запись json: {e}"))?;
            println!("{name}: {}", png_path.display());
        }
    }
    write_atlas(dir)?;
    Ok(())
}

/// T151 / 10.1: вердикт автоматических критериев границ биома.
///
/// Полный объём — 16 сидов × 2 режима при сетке границ ≥200×200 L; такой скан
/// на одном прогоне слишком дорог, поэтому команда принимает подмножество:
/// `--mode` ограничивает режим, `--seed` — один сид, `--seeds 0,42,7` — список.
/// Итоговый вердикт `NO-VOLUME` означает «числа в порядке, но объём не тот»:
/// неполный скан не доказывает выполнение 10.1.
fn cmd_acceptance(args: &[String]) -> Result<(), String> {
    let base = options_from(args)?;
    let Some(model) = height_model_for(args, &base)? else {
        return Err("acceptance требует предпросмотр высоты: --game-data ПУТЬ, \
             --synthetic-height или --version v1-dev"
            .into());
    };
    let opts = height_stats::StatsOptions::parse(args, &base)?;
    let seeds = acceptance_seeds(args, &base)?;
    let modes = acceptance_modes(args, base.mode);
    let json_output = args.iter().any(|arg| arg == "--json");
    let mut summary = acceptance::Summary::default();
    for seed in seeds {
        for mode in modes.iter().copied() {
            let mut run = base.clone();
            run.seed = seed;
            run.mode = mode;
            let stats = height_stats::HeightStats::collect(&run, &opts, &model)?;
            summary.push(acceptance::RunVerdict::from_stats(&stats));
        }
    }
    if json_output {
        print!("{}", summary.json());
    } else {
        print!("{}", summary.text());
    }
    Ok(())
}

/// Сиды для `acceptance`: `--seed` (один), `--seeds 0,42,-7` (список) или весь
/// набор 16 сидов из `SWEEP_SEEDS`.
fn acceptance_seeds(args: &[String], opts: &RenderOptions) -> Result<Vec<i64>, String> {
    if let Some(text) = value(args, "--seeds") {
        let mut seeds = Vec::new();
        for part in text.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            seeds.push(
                part.parse::<i64>()
                    .map_err(|e| format!("--seeds '{part}': {e}"))?,
            );
        }
        if seeds.is_empty() {
            return Err("--seeds: пустой список".into());
        }
        return Ok(seeds);
    }
    if value(args, "--seed").is_some() {
        return Ok(vec![opts.seed]);
    }
    Ok(SWEEP_SEEDS.to_vec())
}

/// Режимы для `acceptance`: явный `--mode` ограничивает один режим, иначе оба.
fn acceptance_modes(args: &[String], default: Mode) -> Vec<Mode> {
    if value(args, "--mode").is_some() {
        return vec![default];
    }
    Mode::ALL.to_vec()
}

/// T123: сырые ингредиенты рельефа по сетке в TSV — для подбора гипсометрии
/// вне генератора (доли полос E, высоты, вклад гидрологии). Только v1/v1-dev.
fn cmd_dump(args: &[String]) -> Result<(), String> {
    let opts = options_from(args)?;
    if !opts.version.is_water_relief_dev() {
        return Err("dump: нужен --version v1-dev".into());
    }
    let span_l = match value(args, "--span-l") {
        Some(text) => text.parse::<f64>().map_err(|e| format!("--span-l: {e}"))?,
        None => 200.0,
    };
    let size = match value(args, "--size") {
        Some(text) => text.parse::<usize>().map_err(|e| format!("--size: {e}"))?,
        None => 32,
    };
    let g = generator(&opts)?;
    let p = g.params().clone();
    let step = span_l * opts.mode.l_blocks() / size as f64;
    let lambda_e = p.block.lambda_e_blocks;
    println!(
        "seed\tmode\tx\tz\tc\te\tw\tpv\ty\triver\tkind\twater\tfloodplain\tswamp\tdry\testuary\tdeferred\te_km\tmountain\tplateau\tiota\ta\tfoothill\tpaleo\tcraton\tescarp\tvolcanic\thills\tbasin\tbasin_km\trift_floor\trift_shoulder\tn_e\tw0\tw1\tw2\tw3\tw4\tw5\tw6\tw7\tt_bio\tratio\twater_y\tbed_y\twater_strength\tch_dist\tch_core\tch_depth\tch_mirror\tch_q\tch_end\tbank"
    );
    for j in 0..size {
        for i in 0..size {
            let x = opts.center_x + (i as f64 - size as f64 * 0.5) * step;
            let z = opts.center_z + (j as f64 - size as f64 * 0.5) * step;
            let Some(world) = g.world_sample_fast(x, z) else {
                continue;
            };
            let v = world.vanilla;
            if v.c < p.climate.ocean_c_threshold {
                continue;
            }
            let relief = g
                .try_relief_sample(x, z)
                .map_err(|e| e.to_string())?
                .ok_or("нет образца рельефа")?;
            let weights = g.morpho_weights(x, z).ok_or("нет морфозон")?;
            let n_e = litho_core::noise::fbm2_unit(
                litho_core::seed::subseed(g.seed(), litho_core::seed::SALT_RELIEF_E),
                x / lambda_e,
                z / lambda_e,
                1.0,
                3,
                0.5,
            );
            let h = world.hydrology;
            let s = world.climate.surface;
            let mut row = format!(
                "{}\t{}\t{x:.1}\t{z:.1}\t{:.4}\t{:.4}\t{:.4}\t{:.4}\t{:.2}\t{:.3}\t{}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.4}\t{:.4}\t{:.4}\t{:.4}",
                opts.seed,
                opts.mode.as_str(),
                v.c,
                v.e,
                v.w,
                world.pv,
                world.height.surface_y,
                world.river,
                h.kind.as_str(),
                h.water,
                h.floodplain,
                h.swamp,
                h.dry_basin,
                h.estuary,
                h.r2_deferred,
                s.e_km,
                s.mountain,
                s.plateau,
                s.iota_l,
            );
            for value in [
                relief.a,
                relief.foothill,
                relief.paleosuture,
                relief.craton,
                relief.escarpment,
                relief.volcanic,
                relief.hills,
                relief.basin,
                relief.basin_depth_km,
                relief.rift_floor,
                relief.rift_shoulder,
                n_e,
            ] {
                row.push_str(&format!("\t{value:.4}"));
            }
            for value in weights.values {
                row.push_str(&format!("\t{value:.3}"));
            }
            row.push_str(&format!(
                "\t{:.2}\t{:.3}",
                world.climate.temperature.t_bio, world.climate.precip.ratio
            ));
            let l = opts.mode.l_blocks();
            row.push_str(&format!(
                "\t{:.2}\t{:.2}\t{:.3}",
                world.height.water_y, world.height.bed_y, world.height.water_strength
            ));
            match h.channels.nearest {
                Some(hit) => row.push_str(&format!(
                    "\t{:.1}\t{:.1}\t{:.2}\t{:.1}\t{:.1}\t{}",
                    hit.distance_l * l,
                    hit.core_l * l,
                    hit.depth_blocks,
                    hit.mirror_y,
                    hit.discharge_mm_l2,
                    hit.end.as_str()
                )),
                None => row.push_str("\t\t\t\t\t\t"),
            }
            row.push_str(&format!("\t{:.3}", h.channels.bank));
            println!("{row}");
        }
    }
    Ok(())
}

fn usage() -> &'static str {
    "Litospherity sandbox (этап Э1, песочница целиком)\n\n\
     Команды:\n\
       render       карты PNG в папку --out\n\
       probe        все поля в точке --at x,z\n\
       dump         T123: ингредиенты рельефа по сетке в TSV (--span-l, --size; v1-dev)\n\
       layers       список слоёв\n\
       stats        метрики 12.4; с --game-data/--synthetic-height — T100\n\
       stats-all    16 сидов × 2 режима; принимает те же флаги высоты\n\
       acceptance   T151/10.1: вердикт границ PASS/FAIL/NO-VOLUME\n\
                    --seeds 0,42,-7 | --seed N | --mode | --json\n\
       seams        скачки C/E/W между соседними блоками\n\
       paths        Д-059: быстрый путь против эталона (μ, P, T/H, высота, зона)\n\
                    --n 48 --span-l 40 --min-zone-share 0.97 --ray-lattice ШАГ_L\n\
       drainage     T110–T116: сток, русла, озёра, дельты и поверхность R1 в --at x,z (v1-dev)\n\
       bench        время эталонного/быстрого пути, колонки и чанка v1-dev\n\
                    --chunk-target-ms N | --enforce-chunk-target\n\
       lod          T79: цена холодных тайлов дальнего рельефа 16 × 16 колонок\n\
                    --strides 4,8,16,32,64,128 --tiles 3 --json\n\
       profile      разбивка времени по слоям\n\
       hash         хеши параметров для замка v0\n\
       checkpoints  10 точек вокруг точки появления\n\
       sweep        16 сидов × 2 режима + HTML-атлас\n\
       atlas        пересобрать только HTML-атлас\n\
       help         эта справка\n\n\
     Общие флаги: --seed N (42), --mode scientific|gameplay (gameplay),\n\
       --version v0|v1|v1-dev (v0), --center x,z (0,0), --size N (512),\n\
       --bpp N, --path reference|fast,\n\
       --layers plates|all|список через запятую, --out ПАПКА\n\
     Слои: шум/плиты/кора, климат, зоны, биомы, пять ванильных чисел,\n\
       реки, лава и точка появления (rivers,lava,spawn); channels/lakes — T112–T114, hydrology — T115/T116, relief — T120 v1-dev.\n\
     T100: --game-data ПУТЬ (распакованные данные игры) или --synthetic-height.\n\
       stats: --span-l 200 --size 64 --relief-size 32 --relief-step 32 --boundary-size 64 --boundary-step 32 --json.\n\
       По умолчанию статистика высоты идёт быстрым путём; --path reference переключает.\n\
       v0: высота приближённая (offset без 3D-шума); синтетика не годится для приёмки.\n\
       v1-dev: собственная высота T144 автоматически; --game-data/--synthetic-height явно переопределяют.\n\
       --pv использует собственное поле PV (T141).\n"
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else {
        print!("{text}", text = usage());
        return Ok(());
    };
    tune::init(&args)?;
    if command == "acceptance" && !tune::label().is_empty() {
        return Err("acceptance считает только Params::defaults; --set здесь запрещён".into());
    }
    match command.as_str() {
        "render" => cmd_render(&args[1..]),
        "probe" => cmd_probe(&args[1..]),
        "dump" => cmd_dump(&args[1..]),
        "stats" => cmd_stats(&args[1..]),
        "stats-all" => cmd_stats_all(&args[1..]),
        "acceptance" => cmd_acceptance(&args[1..]),
        "seams" => cmd_seams(&args[1..]),
        "paths" => cmd_paths(&args[1..]),
        "drainage" => cmd_drainage(&args[1..]),
        "bench" => cmd_bench(&args[1..]),
        "lod" => cmd_lod(&args[1..]),
        "profile" => cmd_profile(),
        "hash" => cmd_hash(),
        "checkpoints" => cmd_checkpoints(&args[1..]),
        "sweep" => cmd_sweep(&args[1..]),
        "atlas" => write_atlas(Path::new(
            &value(&args[1..], "--out").unwrap_or_else(|| "docs/images/sweep".to_string()),
        )),
        "layers" => {
            for layer in LAYERS {
                println!("{layer}");
            }
            Ok(())
        }
        "help" | "--help" | "-h" => {
            print!("{text}", text = usage());
            Ok(())
        }
        other => Err(format!("неизвестная команда '{other}'; попробуйте help")),
    }
}
