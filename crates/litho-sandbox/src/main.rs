//! Песочница Litospherity: карты, метрики, атлас и пробы (план 12.2).

mod palette;
mod png;
mod render;

use std::path::Path;
use std::time::Instant;

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
    })
}

fn generator(opts: &RenderOptions) -> Result<Generator, String> {
    Generator::new(opts.seed, opts.mode, opts.version).map_err(|error| error.to_string())
}

fn cmd_render(args: &[String]) -> Result<(), String> {
    let opts = options_from(args)?;
    let layers = match value(args, "--layers") {
        Some(text) if text == "all" => LAYERS.iter().map(|layer| (*layer).to_string()).collect(),
        Some(text) => text.split(',').map(|s| s.trim().to_string()).collect(),
        None => vec!["plates".to_string()],
    };
    let out = value(args, "--out").unwrap_or_else(|| "out".to_string());
    let dir = Path::new(&out);
    std::fs::create_dir_all(dir).map_err(|e| format!("не могу создать '{out}': {e}"))?;
    let params_hash = Params::defaults(opts.version, opts.mode).params_hash();

    for layer in layers {
        let image = render::render_layer(&opts, &layer)?;
        let png_path = dir.join(format!("{layer}.png"));
        png::write_png(&png_path, &image).map_err(|e| {
            let path = png_path.display();
            format!("запись '{path}': {e}")
        })?;
        let json_path = dir.join(format!("{layer}.json"));
        let meta_opts = render::options_for_layer(&opts, &layer);
        std::fs::write(
            &json_path,
            render::metadata(&meta_opts, &layer, params_hash),
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
fn cmd_stats_all() -> Result<(), String> {
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

fn cmd_bench(args: &[String]) -> Result<(), String> {
    let opts = options_from(args)?;
    let generator = generator(&opts)?;
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

fn usage() -> &'static str {
    "Litospherity sandbox (этап Э1, песочница целиком)\n\n\
     Команды:\n\
       render       карты PNG в папку --out\n\
       probe        все поля в точке --at x,z\n\
       layers       список слоёв\n\
       stats        метрики 12.4 по области 200×200 L\n\
       stats-all    те же метрики на 16 сидах × 2 режима\n\
       seams        скачки C/E/W между соседними блоками\n\
       bench        время эталонного и быстрого пути\n\
       profile      разбивка времени по слоям\n\
       checkpoints  10 точек вокруг точки появления\n\
       sweep        16 сидов × 2 режима + HTML-атлас\n\
       atlas        пересобрать только HTML-атлас\n\
       help         эта справка\n\n\
     Общие флаги: --seed N (42), --mode scientific|gameplay (gameplay),\n\
       --version v0|v1 (v0), --center x,z (0,0), --size N (512),\n\
       --bpp N, --path reference|fast,\n\
       --layers plates|all|список через запятую, --out ПАПКА\n\
     Слои: шум/плиты/кора, климат, зоны, биомы, пять ванильных чисел,\n\
       реки, лава и точка появления (rivers,lava,spawn).\n"
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else {
        print!("{text}", text = usage());
        return Ok(());
    };
    match command.as_str() {
        "render" => cmd_render(&args[1..]),
        "probe" => cmd_probe(&args[1..]),
        "stats" => cmd_stats(&args[1..]),
        "stats-all" => cmd_stats_all(),
        "seams" => cmd_seams(&args[1..]),
        "bench" => cmd_bench(&args[1..]),
        "profile" => cmd_profile(),
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
