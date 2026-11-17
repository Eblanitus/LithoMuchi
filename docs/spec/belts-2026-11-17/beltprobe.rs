//! Расследование: биом против широтного пояса.
//! beltprobe <seed> <gameplay|scientific> <n> <half_span_L> [threads]
use litho_core::classify::{classify, classify_inputs, ClassSample};
use litho_core::climate::{dryness, sea_level_temp, temperature_cooled};
use litho_core::generator::Generator;
use litho_core::mode::Mode;
use litho_core::version::GenVersion;
use litho_core::{ClimateSample, Params, SurfaceKind};

#[derive(Clone, Default)]
struct Rec {
    x: f64,
    z: f64,
    phi: f64,
    cont: f64,
    y: f64,
    shape_cool_extra: f64,
    belt_ref: u8,
    belts: [u8; 5],
    ts: [u8; 5],
    names: [&'static str; 5],
    t_min_ref: f64,
    t_min_fin: f64,
    t_bio_ref: f64,
    t_bio_fin: f64,
    province: u8,
    kind: u8,
    vt: f64,
    ve: f64,
    vw: f64,
    vc: f64,
    biome: &'static str,
    t_nowinter: u8,
    belt_delta_only: u8,
    belt_amp_only: u8,
    tmin_amp_only: f64,
}

fn allowed(belt: u8) -> (u8, u8) {
    match belt {
        0 | 1 => (0, 0),
        2 => (1, 1),
        3 => (2, 2),
        4 => (2, 4),
        _ => (3, 4),
    }
}

fn names_for_t(t: u8) -> &'static [&'static str] {
    match t {
        0 => &["snowy_plains", "snowy_taiga", "taiga", "frozen_river", "snowy_beach"],
        1 => &["plains", "forest", "taiga", "old_growth_spruce_taiga", "river", "swamp"],
        2 => &["flower_forest", "plains", "forest", "birch_forest", "dark_forest", "river", "swamp"],
        3 => &["savanna", "forest", "jungle", "river", "mangrove_swamp"],
        _ => &["desert", "river", "mangrove_swamp"],
    }
}

/// true, если имя биома не встречается ни в одной допустимой строке пояса.
fn visible_bad(belt: u8, name: &str) -> bool {
    let (lo, hi) = allowed(belt);
    !(lo..=hi).any(|t| names_for_t(t).contains(&name))
}

fn stage(phi: f64, cool: f64, cont: f64, p_mm: f64, p: &Params) -> (ClassSample, f64, f64) {
    let t = temperature_cooled(phi, cool, cont, p);
    let (_, ratio) = dryness(t.t_bio, p_mm, p);
    (classify_inputs(t.t_bio, ratio, t.t_min, p), t.t_min, t.t_bio)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let seed: i64 = a[1].parse().unwrap();
    let mode = if a[2] == "scientific" { Mode::Scientific } else { Mode::Gameplay };
    let n: i64 = a[3].parse().unwrap();
    let half_l: f64 = a[4].parse().unwrap();
    let threads: usize = a.get(5).map(|s| s.parse().unwrap()).unwrap_or(8);
    let g0 = Generator::new(seed, mode, GenVersion::V1Dev).unwrap();
    let lb = g0.params().block.l_blocks;
    let half = half_l * lb;
    let step = 2.0 * half / n as f64;
    let rows: Vec<i64> = (0..n).collect();
    let chunks: Vec<Vec<i64>> = rows.chunks((n as usize).div_ceil(threads)).map(|c| c.to_vec()).collect();
    let mut recs: Vec<Rec> = Vec::new();
    let mut ocean = 0usize;
    std::thread::scope(|s| {
        let hs: Vec<_> = chunks
            .into_iter()
            .map(|rows| {
                s.spawn(move || {
                    let g = Generator::new(seed, mode, GenVersion::V1Dev).unwrap();
                    let p = g.params().clone();
                    let useed = litho_core::seed::from_world_seed(seed);
                    let mut out = Vec::new();
                    let mut oc = 0usize;
                    for j in rows {
                        for i in 0..n {
                            let x = -half + (i as f64 + 0.5) * step;
                            let z = -half + (j as f64 + 0.5) * step;
                            let Some(f) = g.sample(x, z) else { continue };
                            let w = &f.world;
                            if w.is_ocean {
                                oc += 1;
                                continue;
                            }
                            let cl: &ClimateSample = &w.climate;
                            let phi = cl.pole.phi_deg;
                            let cont = cl.precip.cont;
                            let p_mm = cl.precip.p_mm;
                            // полный вычет, который применил recool
                            let t_nocool = temperature_cooled(phi, 0.0, cont, &p).t_c;
                            let cool_total = t_nocool - cl.temperature.t_c;
                            let (c0, tmin0, tbio0) = stage(phi, p.climate.alt_cool_land_c, 0.0, p_mm, &p);
                            let (c1, _, _) = stage(phi, p.climate.alt_cool_land_c, cont, p_mm, &p);
                            let (c2, _, _) = stage(phi, cool_total, cont, p_mm, &p);
                            let c3 = classify(cl, false, x, z, useed, &p);
                            let c4 = w.class;
                            let kind = match w.hydrology.kind {
                                SurfaceKind::Land => 0,
                                SurfaceKind::River => 1,
                                SurfaceKind::Lake => 2,
                                SurfaceKind::Swamp => 3,
                                SurfaceKind::DryBasin => 4,
                                SurfaceKind::DeferredWater => 5,
                                SurfaceKind::InlandSea => 6,
                                #[allow(unreachable_patterns)]
                                _ => 7,
                            };
                            let _ = sea_level_temp;
                            out.push(Rec {
                                x,
                                z,
                                phi,
                                cont,
                                y: w.height.surface_y,
                                shape_cool_extra: cool_total - p.climate.alt_cool_land_c,
                                belt_ref: c0.zone.belt,
                                belts: [c0.zone.belt, c1.zone.belt, c2.zone.belt, c3.zone.belt, c4.zone.belt],
                                ts: [c0.cell.t, c1.cell.t, c2.cell.t, c3.cell.t, c4.cell.t],
                                names: [c0.cell.biome, c1.cell.biome, c2.cell.biome, c3.cell.biome, c4.cell.biome],
                                t_min_ref: tmin0,
                                t_min_fin: cl.temperature.t_min,
                                t_bio_ref: tbio0,
                                t_bio_fin: c3.t_bio,
                                province: c3.zone.province,
                                kind,
                                vt: w.vanilla.t,
                                ve: w.vanilla.e,
                                vw: w.vanilla.w,
                                vc: w.vanilla.c,
                                biome: c4.cell.biome,
                                belt_delta_only: {
                                    let c = &p.climate;
                                    let ss = litho_core::math::smoothstep;
                                    let delta = cont * (c.cont_tropical_c * (1.0 - ss(25.0, 45.0, phi)) - c.cont_polar_c * ss(40.0, 70.0, phi));
                                    let amp_lat = c.amp_base_c + c.amp_lat_c * ss(5.0, 75.0, phi);
                                    let t_c = sea_level_temp(phi, &p) + delta - c.alt_cool_land_c;
                                    let (bio, _) = litho_core::climate::seasonal(t_c, amp_lat * c.amp_sea_factor, &p);
                                    classify_inputs(bio, 1.0, 0.0, &p).zone.belt
                                },
                                belt_amp_only: {
                                    let c = &p.climate;
                                    let ss = litho_core::math::smoothstep;
                                    let amp_lat = c.amp_base_c + c.amp_lat_c * ss(5.0, 75.0, phi);
                                    let t_c = sea_level_temp(phi, &p) - c.alt_cool_land_c;
                                    let (bio, _) = litho_core::climate::seasonal(t_c, amp_lat * (c.amp_sea_factor + c.amp_land_factor * cont), &p);
                                    classify_inputs(bio, 1.0, 0.0, &p).zone.belt
                                },
                                tmin_amp_only: 0.0,
                                t_nowinter: litho_core::classify::biome_cell_at(c3.zone.belt, c3.zone.province, c3.t_bio, c3.ratio, 50.0, &p).t,
                            });
                        }
                    }
                    (out, oc)
                })
            })
            .collect();
        for h in hs {
            let (o, oc) = h.join().unwrap();
            recs.extend(o);
            ocean += oc;
        }
    });
    report(&recs, ocean, seed, &a[2], step);
}

const STAGE: [&str; 5] = [
    "S0 правило таблицы (эталон: широта, уровень моря, морской климат)",
    "S1 континентальность (лето/зима, ±°C)",
    "S2 высота К5 (выше y 120)",
    "S3 шум-дрожание границ",
    "S4 накладки воды (река, болото, котловина, галерея)",
];

fn report(recs: &[Rec], ocean: usize, seed: i64, mode: &str, step: f64) {
    let n = recs.len() as f64;
    println!("=== seed {seed} {mode}: суша {} точек, океан {ocean}, шаг {step:.0} блоков", recs.len());
    // распределение эталонных поясов
    let mut bref = [0usize; 7];
    let mut bfin = [0usize; 7];
    for r in recs {
        bref[r.belt_ref as usize] += 1;
        bfin[r.belts[3] as usize] += 1;
    }
    println!("пояс по широте (эталон), %: {}", (0..7).map(|b| format!("П{b} {:.1}", 100.0 * bref[b] as f64 / n)).collect::<Vec<_>>().join(", "));
    println!("пояс в коде (итог),     %: {}", (0..7).map(|b| format!("П{b} {:.1}", 100.0 * bfin[b] as f64 / n)).collect::<Vec<_>>().join(", "));
    // сдвиг пояса по стадиям
    let mut shift = [0usize; 4];
    let mut shift_any = 0usize;
    let mut shift_up = 0usize;
    let mut shift_dn = 0usize;
    for r in recs {
        for s in 1..4 {
            if r.belts[s] != r.belts[s - 1] {
                shift[s] += 1;
            }
        }
        if r.belts[3] != r.belt_ref {
            shift_any += 1;
            if r.belts[3] > r.belt_ref { shift_up += 1 } else { shift_dn += 1 }
        }
    }
    println!(
        "пояс в коде ≠ пояс по широте: {:.1} % суши (теплее {:.1}, холоднее {:.1}); меняют пояс: континентальность {:.1}, высота {:.1}, шум {:.1}",
        100.0 * shift_any as f64 / n,
        100.0 * shift_up as f64 / n,
        100.0 * shift_dn as f64 / n,
        100.0 * shift[1] as f64 / n,
        100.0 * shift[2] as f64 / n,
        100.0 * shift[3] as f64 / n
    );
    // нарушения: итоговый t вне допустимого для широтного пояса
    let mut viol = 0usize;
    let mut invisible = 0usize;
    let mut hot = 0usize;
    let mut cold = 0usize;
    let mut by_stage = [[0usize; 2]; 5]; // [смена пояса, правило без смены пояса]
    let mut examples: Vec<Vec<&Rec>> = vec![Vec::new(); 10];
    let mut sub: std::collections::BTreeMap<String, usize> = Default::default();
    for r in recs {
        let (lo, hi) = allowed(r.belt_ref);
        let t = r.ts[4];
        if t >= lo && t <= hi {
            continue;
        }
        viol += 1;
        if t > hi { hot += 1 } else { cold += 1 }
        // самая ранняя стадия, с которой t уже вне нормы до конца
        let mut s = 4;
        while s > 0 {
            let tp = r.ts[s - 1];
            let same_side = if t > hi { tp > hi } else { tp < lo };
            if !same_side { break; }
            s -= 1;
        }
        if !visible_bad(r.belt_ref, r.biome) { invisible += 1; }
        let belt_changed = s > 0 && r.belts[s] != r.belts[s - 1];
        by_stage[s][if belt_changed { 0 } else { 1 }] += 1;
        let dir = if t > hi { "жарче" } else { "холоднее" };
        let key = format!(
            "{} | {} | П{}→{} ({})",
            &STAGE[s][..2],
            dir,
            r.belt_ref,
            r.biome,
            if belt_changed { "сдвиг пояса" } else { "правило" }
        );
        *sub.entry(key).or_default() += 1;
        let k = s * 2 + usize::from(belt_changed);
        examples[k].push(r);
    }
    println!(
        "НАРУШЕНИЯ: {:.1} % суши (жарче пояса {:.1}, холоднее {:.1})",
        100.0 * viol as f64 / n,
        100.0 * hot as f64 / n,
        100.0 * cold as f64 / n
    );
    println!("  из них имя биома допустимо для пояса (видимой разницы нет, только ванильная строка T): {:.1} % суши", 100.0 * invisible as f64 / n);
    for s in 0..5 {
        println!(
            "  {}: {:.2} % (из них со сдвигом пояса {:.2}, правилом {:.2})",
            STAGE[s],
            100.0 * (by_stage[s][0] + by_stage[s][1]) as f64 / n,
            100.0 * by_stage[s][0] as f64 / n,
            100.0 * by_stage[s][1] as f64 / n
        );
    }
    let mut subv: Vec<_> = sub.into_iter().collect();
    subv.sort_by(|a, b| b.1.cmp(&a.1));
    println!("  разбивка (топ 25):");
    for (k, c) in subv.iter().take(25) {
        println!("    {:6.2} %  {k}", 100.0 * *c as f64 / n);
    }
    // примеры
    for (k, ex) in examples.iter().enumerate() {
        if ex.is_empty() { continue; }
        let s = k / 2;
        println!("  примеры {} {}:", &STAGE[s][..2], if k % 2 == 1 { "(сдвиг пояса)" } else { "(правило)" });
        let stride = (ex.len() / 4).max(1);
        for r in ex.iter().step_by(stride).take(4) {
            println!(
                "    X {:.0} Z {:.0}: φ {:.1}°, cont {:.2}, y {:.0}, вычет сверх 3,5 = {:.1} °C; T_bio эталон {:.1} → итог {:.1}, T_min эталон {:.1} → итог {:.1}; пояс П{} → П{}; K{}; биом эталона {} → итог {} (кинд {})",
                r.x, r.z, r.phi, r.cont, r.y, r.shape_cool_extra, r.t_bio_ref, r.t_bio_fin, r.t_min_ref, r.t_min_fin, r.belt_ref, r.belts[3], r.province, r.names[0], r.biome, r.kind
            );
        }
    }
    {
        let d = recs.iter().filter(|r| r.belt_delta_only != r.belt_ref).count();
        let a = recs.iter().filter(|r| r.belt_amp_only != r.belt_ref).count();
        let a_up = recs.iter().filter(|r| r.belt_amp_only > r.belt_ref).count();
        let warm1: Vec<&Rec> = recs.iter().filter(|r| r.belts[1] > r.belts[0]).collect();
        let disjoint = |a: u8, b: u8| { let (l1, h1) = allowed(a); let (l2, h2) = allowed(b); h1 < l2 || h2 < l1 };
        let warm1: Vec<&Rec> = warm1.into_iter().filter(|r| disjoint(r.belts[0], r.belts[1])).collect();
        let back = warm1.iter().filter(|r| { let (lo, hi) = allowed(r.belts[0]); r.ts[1] >= lo && r.ts[1] <= hi }).count();
        println!("пояс стал теплее со сменой ванильной строки: {:.1} % суши", 100.0 * warm1.len() as f64 / n);
        let norecool = recs.iter().filter(|r| r.shape_cool_extra < -0.01).count();
        println!(
            "континентальность: только сдвиг среднего (±°C) меняет пояс у {:.1} %, только размах лето/зима у {:.1} % (теплее {:.1} %); из точек, где пояс стал теплее, ванильная строка T не изменилась у {:.0} % (зима вернула назад); суша без пересчёта К5 (устья у моря): {:.1} %",
            100.0 * d as f64 / n, 100.0 * a as f64 / n, 100.0 * a_up as f64 / n,
            100.0 * back as f64 / warm1.len().max(1) as f64, 100.0 * norecool as f64 / n
        );
    }
    // УРОВЕНЬ 1: биом против пояса, который посчитал сам код (после всех поправок)
    let mut l1 = 0usize;
    let mut l1_vis = 0usize;
    let mut l1c: std::collections::BTreeMap<String, usize> = Default::default();
    let mut l1ex: std::collections::BTreeMap<String, Vec<&Rec>> = Default::default();
    for r in recs {
        let b = r.belts[3];
        let (lo, hi) = allowed(b);
        let t = r.ts[4];
        if t >= lo && t <= hi { continue; }
        l1 += 1;
        if visible_bad(b, r.biome) { l1_vis += 1; }
        let t3 = r.ts[3];
        let dir = if t > hi { "жарче" } else { "холоднее" };
        let cause = if t3 >= lo && t3 <= hi {
            format!("накладка воды: {}", r.biome)
        } else if r.t_nowinter >= lo && r.t_nowinter <= hi {
            format!("зима (T_min) в П{b}: {}", r.biome)
        } else {
            format!("таблица (пустыня/кайма/сухо) в П{b}: {}", r.biome)
        };
        let key = format!("{dir} | {cause}");
        *l1c.entry(key.clone()).or_default() += 1;
        l1ex.entry(key).or_default().push(r);
    }
    println!("УРОВЕНЬ 1 (биом против пояса самого кода): {:.1} % суши, из них видимо по имени {:.1} %", 100.0 * l1 as f64 / n, 100.0 * l1_vis as f64 / n);
    let mut v: Vec<_> = l1c.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1));
    for (k, c) in v.iter().take(20) {
        let ex = &l1ex[*k];
        let r = ex[ex.len() / 2];
        println!("    {:6.2} %  {k}   напр. X {:.0} Z {:.0} (y {:.0}, T_min {:.1})", 100.0 * **c as f64 / n, r.x, r.z, r.y, r.t_min_fin);
    }
    let mut kinds = [0usize; 8];
    for r in recs { kinds[r.kind as usize] += 1; }
    println!("виды поверхности суши, %: суша {:.1}, река {:.1}, озеро {:.1}, болото {:.1}, котловина {:.1}, отложенная вода {:.1}, внутр. море {:.1}", 
        100.0*kinds[0] as f64/n,100.0*kinds[1] as f64/n,100.0*kinds[2] as f64/n,100.0*kinds[3] as f64/n,100.0*kinds[4] as f64/n,100.0*kinds[5] as f64/n,100.0*kinds[6] as f64/n);
    let rivers: Vec<&Rec> = recs.iter().filter(|r| (r.biome == "river" || r.biome == "frozen_river") && r.belts[3] >= 2 && r.belts[3] <= 4).collect();
    let fr = rivers.iter().filter(|r| r.biome == "frozen_river").count();
    println!("реки в поясах П2–П4: замёрзших {:.0} % ({} из {})", 100.0 * fr as f64 / rivers.len().max(1) as f64, fr, rivers.len());
    // К5: доля суши выше y 120 и что там
    let hi: Vec<&Rec> = recs.iter().filter(|r| r.shape_cool_extra > 0.01).collect();
    let hi_changed = hi.iter().filter(|r| r.ts[2] != r.ts[1]).count();
    let hi_belt = hi.iter().filter(|r| r.belts[2] != r.belts[1]).count();
    let band = |lo: f64, hi_: f64| {
        let v: Vec<&&Rec> = hi.iter().filter(|r| { let y = 120.0 + r.shape_cool_extra / 0.2; y >= lo && y < hi_ }).collect();
        let c = v.iter().filter(|r| r.ts[2] < r.ts[1]).count();
        (v.len(), c)
    };
    println!(
        "К5: суша с формой выше y 120: {:.1} %; из неё пояс сменён у {:.1} %, ванильная ступень T снижена у {:.1} %",
        100.0 * hi.len() as f64 / n,
        100.0 * hi_belt as f64 / hi.len().max(1) as f64,
        100.0 * hi_changed as f64 / hi.len().max(1) as f64
    );
    for (lo, h) in [(120.0, 140.0), (140.0, 170.0), (170.0, 220.0), (220.0, 1000.0)] {
        let (c, d) = band(lo, h);
        println!("   y {lo:.0}–{h:.0}: {:.2} % суши, T ниже у {:.0} % из них", 100.0 * c as f64 / n, 100.0 * d as f64 / c.max(1) as f64);
    }
    // Ваниль: кандидаты в снежные склоны/пики (оценка по полосам 1.18+)
    let e_bin = |e: f64| -> usize {
        let edges = [-0.78, -0.375, -0.2225, 0.05, 0.45, 0.55];
        edges.iter().take_while(|v| e >= **v).count()
    };
    let c_bin = |c: f64| -> usize {
        // 0 coast(-0.19..-0.11), 1 near, 2 mid, 3 far
        if c < -0.11 { 0 } else if c < 0.03 { 1 } else if c < 0.3 { 2 } else { 3 }
    };
    let t_bin = |t: f64| -> u8 {
        let edges = [-0.45, -0.15, 0.2, 0.55];
        edges.iter().take_while(|v| t >= **v).count() as u8
    };
    let mut snow = 0usize;
    let mut snow_warm = 0usize; // при T1/T2
    let mut ys: Vec<f64> = Vec::new();
    let mut snow_ex: Vec<&Rec> = Vec::new();
    for r in recs {
        if r.kind != 0 { continue; }
        let aw = r.vw.abs();
        let e = e_bin(r.ve);
        let c = c_bin(r.vc);
        let t = t_bin(r.vt);
        let peak_slice = aw >= 0.5667 && aw < 0.7667;
        let high_slice = (aw >= 0.4 && aw < 0.5667) || (aw >= 0.7667 && aw < 0.9333);
        let mid_slice = (aw >= 0.2667 && aw < 0.4) || aw >= 0.9333;
        let cand = (peak_slice && (e == 0 || (e == 1 && c >= 2) || (e == 1 && t == 0)))
            || (high_slice && ((e == 0 && c >= 1) || (e == 1 && c >= 2) || (e == 1 && c == 1 && t == 0)))
            || (mid_slice && e == 0 && c >= 1);
        if cand && t <= 2 {
            snow += 1;
            if t >= 1 {
                snow_warm += 1;
                ys.push(r.y);
                snow_ex.push(r);
            }
        }
    }
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let q = |f: f64| if ys.is_empty() { f64::NAN } else { ys[((ys.len() - 1) as f64 * f) as usize] };
    println!(
        "ВАНИЛЬ (оценка): снежные склоны/пики ванили (T≤2, E0–E1, высокий |W|) на {:.2} % суши; из них при поясе t1–t2 (не полярном) {:.2} %; высота у них: p10 {:.0}, медиана {:.0}, p90 {:.0}",
        100.0 * snow as f64 / n,
        100.0 * snow_warm as f64 / n,
        q(0.1),
        q(0.5),
        q(0.9)
    );
    let stride = (snow_ex.len() / 3).max(1);
    for r in snow_ex.iter().step_by(stride).take(3) {
        println!("    X {:.0} Z {:.0}: y {:.0}, биом ядра {}, E {:.2}, W {:.2}, C {:.2}", r.x, r.z, r.y, r.biome, r.ve, r.vw, r.vc);
    }
    // таблица: эталонный пояс × итоговая ванильная ступень t
    println!("матрица П(широта) × t(итог), % суши:");
    for b in 0..7u8 {
        let row: Vec<String> = (0..5u8)
            .map(|t| {
                let c = recs.iter().filter(|r| r.belt_ref == b && r.ts[4] == t).count();
                format!("{:5.1}", 100.0 * c as f64 / n)
            })
            .collect();
        println!("  П{b}: t0..t4 = {}", row.join(" "));
    }
}
