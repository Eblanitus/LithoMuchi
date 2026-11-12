//! Метрики 10.1, которых не было в отчёте `final1`: «климат влияет на форму»
//! и строки «Реки и озёра» (кроме воды в русле).
//!
//! Всё считается по той же сетке, что основная гипсометрия `stats`, и только в
//! `v1-dev`: у `v0` нет ни морфоклиматических зон, ни сети русел, ни озёр.
//! Это диагностика, а не приёмка: приёмочный вердикт по-прежнему требует
//! полного объёма 10.1 и живёт в `acceptance`.

use litho_core::morphoclimate::ZONE_NAMES;
use litho_core::params::MORPHO_ZONES;

/// Полоса E считается «своей» для зоны, если она отличается от средней по
/// суше на столько процентных пунктов (10.1).
pub const CLIMATE_SHAPE_MIN_GAP_PP: f64 = 10.0;
/// Аридная зона: доля E2 (10.1).
pub const ARID_E2_MIN_PCT: f64 = 20.0;
/// Бореальная и полярная: озёр не меньше, чем в столько раз выше среднего.
pub const COLD_LAKE_FACTOR: f64 = 2.0;
/// Влажная тропическая: плотность рек не меньше, чем в столько раз выше.
pub const TROPICAL_RIVER_FACTOR: f64 = 1.5;

/// Границы полос E для отчёта (совпадают с `height_stats::E_EDGES`).
const E_EDGES: [f64; 6] = [-0.78, -0.375, -0.2225, 0.05, 0.45, 0.55];

/// Одна зона 6.3 в отчёте.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZoneShape {
    pub name: &'static str,
    /// Точек суши, где зона доминирует.
    pub land: usize,
    /// Доли полос E 0…6 в зоне, %.
    pub erosion_pct: [f64; 7],
    /// Доля точек зоны с водой озера, %.
    pub lake_pct: f64,
    /// Доля точек зоны в русле, %.
    pub river_pct: f64,
    /// Полосы E, отличающиеся от средней по суше на ≥ 10 п.п.
    pub distinct_bands: usize,
    /// Самое большое отличие от средней по суше, п.п.
    pub max_gap_pp: f64,
}

/// «Климат влияет на форму» (10.1).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClimateShape {
    /// Средние доли полос E по всей суше, %: эталон для сравнения зон.
    pub erosion_pct: [f64; 7],
    /// Средняя доля точек суши с водой озера, %.
    pub lake_pct: f64,
    /// Средняя доля точек суши в русле, % (плотность сети в этой сетке).
    pub river_pct: f64,
    pub zones: Vec<ZoneShape>,
    pub land: usize,
    /// Итоговая проверка 10.1 по зонам, где есть достаточный объём.
    /// `None` — объём меньше порога, вердикта нет.
    pub pass: Option<bool>,
    /// Почему проверка не прошла или не считается.
    pub notes: Vec<String>,
}

/// Строки «Реки и озёра» 10.1, которые считаются по сетке `stats`.
///
/// Часть строк плана требует не сетки, а обхода сети русел (где кончается
/// река, ширина устья, длинные реки, озёра на 100 L²) — они живут в
/// [`RiverReport`] и считаются из одной сети на плиту.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RiverLake {
    /// Точек суши в основной сетке.
    pub land: usize,
    /// Точек суши с водой озера (`hydrology.water ≥ 0.5`).
    pub lake_land: usize,
    /// Точек суши в русле (`channels.bank ≥ 0.5`).
    pub river_land: usize,
    /// Точек суши вне рек и озёр — для «климата» и прочих долей.
    pub dry_land: usize,
    /// Площадь области в L², по которой считаются плотности.
    pub area_l2: f64,
}

impl RiverLake {
    pub fn lake_pct(&self) -> f64 {
        pct(self.lake_land, self.land)
    }
    pub fn river_pct(&self) -> f64 {
        pct(self.river_land, self.land)
    }
    /// Озеро на 100 L² суши (10.1). `lake_land` — точки воды на сетке,
    /// поэтому «озеро» здесь — единица площади воды, а не отдельный водоём:
    /// по сетке отдельные озёра не различить. Счёт водоёмов — в `drainage`.
    pub fn lakes_per_100_l2(&self) -> f64 {
        if self.land == 0 || self.area_l2 <= 0.0 {
            0.0
        } else {
            let land_l2 =
                self.area_l2 * self.land as f64 / (self.land + self.lake_land).max(1) as f64;
            100.0 * self.lake_land as f64 / land_l2.max(1e-9)
        }
    }
}

fn pct(count: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        100.0 * count as f64 / total as f64
    }
}

fn e_band(e: f64) -> usize {
    E_EDGES.partition_point(|edge| e >= *edge)
}

/// Сколько точек зоны нужно, чтобы вердикт по зоне имел смысл. Меньше —
/// зона попала в выборку краем и в вердикт не идёт, только в таблицу.
pub const MIN_ZONE_LAND: usize = 50;

/// Накопитель по основной сетке: `stats` отдаёт ему уже посчитанную точку,
/// чтобы не звать `world_sample` второй раз (это удвоило бы время замера).
#[derive(Clone, Debug, Default)]
pub struct PlanAccumulator {
    erosion: [usize; 7],
    zone_land: [usize; MORPHO_ZONES],
    zone_erosion: [[usize; 7]; MORPHO_ZONES],
    zone_lake: [usize; MORPHO_ZONES],
    zone_river: [usize; MORPHO_ZONES],
    dry_land: usize,
    lake_land: usize,
    river_land: usize,
    diag: ZoneDiagnostics,
}

impl PlanAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Одна точка основной сетки. `dominant` — доминирующая морфозона (её
    /// считает генератор для формы рельефа), `e` — эрозия в игре, `in_river`
    /// и `in_lake` — узкое русло и вода озера.
    #[allow(clippy::too_many_arguments)]
    pub fn add(&mut self, land: bool, e: f64, in_river: bool, in_lake: bool, dominant: usize) {
        if !land && !in_river && !in_lake {
            return;
        }
        if in_river {
            self.river_land += 1;
        }
        if in_lake {
            self.lake_land += 1;
        }
        if !land || in_river || in_lake {
            return;
        }
        self.dry_land += 1;
        let band = e_band(e);
        self.erosion[band] += 1;
        if dominant < MORPHO_ZONES {
            self.zone_land[dominant] += 1;
            self.zone_erosion[dominant][band] += 1;
        }
    }

    /// Диагностика T123 по этой же выборке.
    pub fn diagnose(&mut self, mountain: f64, plateau: f64, m_e: f64) {
        self.diag.add(mountain, plateau, m_e);
    }

    /// Замыкает: считает доли и вердикт 10.1 по зонам.
    pub fn finish(self, area_l2: f64) -> (ClimateShape, RiverLake, ZoneDiagnostics) {
        let mut shape = ClimateShape::default();
        let rivers = RiverLake {
            land: self.dry_land + self.lake_land + self.river_land,
            dry_land: self.dry_land,
            lake_land: self.lake_land,
            river_land: self.river_land,
            area_l2,
        };
        shape.land = self.dry_land;
        if self.dry_land == 0 {
            return (shape, rivers, self.diag);
        }
        for (band, count) in self.erosion.iter().enumerate() {
            shape.erosion_pct[band] = pct(*count, self.dry_land);
        }
        shape.lake_pct = pct(self.lake_land, rivers.land);
        shape.river_pct = pct(self.river_land, rivers.land);

        let mut all_ok = true;
        let mut any_zone_verdict = false;
        for (zone, name) in ZONE_NAMES.iter().enumerate() {
            let total = self.zone_land[zone] + self.zone_lake[zone] + self.zone_river[zone];
            let mut z = ZoneShape {
                name,
                land: total,
                ..ZoneShape::default()
            };
            if total == 0 {
                shape.zones.push(z);
                continue;
            }
            for (band, value) in z.erosion_pct.iter_mut().enumerate() {
                *value = pct(self.zone_erosion[zone][band], self.zone_land[zone].max(1));
            }
            z.lake_pct = pct(self.zone_lake[zone], total);
            z.river_pct = pct(self.zone_river[zone], total);
            for (here, all) in z.erosion_pct.iter().zip(shape.erosion_pct.iter()) {
                let gap = (here - all).abs();
                z.max_gap_pp = z.max_gap_pp.max(gap);
                if gap >= CLIMATE_SHAPE_MIN_GAP_PP {
                    z.distinct_bands += 1;
                }
            }
            let mut zone_ok = true;
            if total < MIN_ZONE_LAND {
                shape.notes.push(format!(
                    "{}: {} точек < {MIN_ZONE_LAND}, в вердикт не идёт",
                    z.name, total
                ));
                shape.zones.push(z);
                continue;
            }
            any_zone_verdict = true;
            if z.distinct_bands == 0 {
                zone_ok = false;
            }
            if z.name == "arid" && z.erosion_pct[2] < ARID_E2_MIN_PCT {
                zone_ok = false;
                shape.notes.push(format!(
                    "arid: E2 {:.1} % < {ARID_E2_MIN_PCT} %",
                    z.erosion_pct[2]
                ));
            }
            if (z.name == "polar" || z.name == "boreal")
                && z.lake_pct < COLD_LAKE_FACTOR * shape.lake_pct
            {
                zone_ok = false;
                shape.notes.push(format!(
                    "{}: озёра {:.2} % < {COLD_LAKE_FACTOR}× среднего {:.2} %",
                    z.name, z.lake_pct, shape.lake_pct
                ));
            }
            if z.name == "tropical_humid" && z.river_pct < TROPICAL_RIVER_FACTOR * shape.river_pct {
                zone_ok = false;
                shape.notes.push(format!(
                    "tropical_humid: реки {:.2} % < {TROPICAL_RIVER_FACTOR}× среднего {:.2} %",
                    z.river_pct, shape.river_pct
                ));
            }
            all_ok &= zone_ok;
            shape.zones.push(z);
        }
        shape.pass = if any_zone_verdict { Some(all_ok) } else { None };
        (shape, rivers, self.diag)
    }
}

/// T123: `m_e` — вес гор в переводе E (`relief_fields::sample`).
/// Повторяет формулу ядра, чтобы диагностика показывала то же число.
pub fn mountain_weight(mountain: f64, params: &litho_core::Params) -> f64 {
    let c = &params.climate;
    smoothstep(c.m_e_lo, c.m_e_hi, mountain)
}

fn smoothstep(lo: f64, hi: f64, v: f64) -> f64 {
    if hi <= lo {
        return if v >= hi { 1.0 } else { 0.0 };
    }
    let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Итог по одной сети русел: строки 10.1, которые сеткой не считаются.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RiverReport {
    pub plate_id: u64,
    /// Цепочек русел всего (включая рукава дельт).
    pub reaches: usize,
    /// Устьев с водой на конце (море, рифт, трансформ, озеро).
    pub ends_with_water: usize,
    /// Устьев без выхода (ниже Q_min, граница, замкнутый избыток).
    pub ends_unresolved: usize,
    /// Устьев в бессточную котловину — это допустимый конец по 10.1.
    pub ends_endorheic: usize,
    /// Устьев в озеро.
    pub ends_lake: usize,
    /// Устьев в море/рифт/трансформ (межплитовая вода).
    pub ends_sea: usize,
    /// Устьев в другую реку (слияние).
    pub ends_confluence: usize,
    /// Медиана извилистости равнинных русел.
    pub plain_sinuosity_median: Option<f64>,
    /// Самая длинная цепочка, L.
    pub max_reach_l: f64,
    /// Устья, у которых ширина хотя бы вдвое больше истока.
    pub wider_mouth: usize,
    /// Русел длиннее 5 L, у которых ширину можно сравнить.
    pub long_reaches: usize,
    /// Озёр с водой на плите.
    pub lakes: usize,
    /// Площадь зеркал, L².
    pub lake_area_l2: f64,
    /// Озёр выше 63 блоков (R2 отложена, но метрика 10.1 спрашивает).
    pub high_lakes: usize,
}

/// Собирает [`RiverReport`] прямо из построенной сети плиты: цепочки, устья,
/// извилистость, ширина вниз по течению и озёра.
///
/// Ширина сравнивается по `core_l` первого и последнего отрезка цепочки:
/// устье должно быть хотя бы вдвое шире истока (10.1).
pub fn report_from_net(net: &litho_core::drainage::RiverNet) -> RiverReport {
    let channels = net.channels();
    let mut rows = Vec::with_capacity(channels.reaches().len());
    for (index, reach) in channels.reaches().iter().enumerate() {
        let segments = channels.reach_segments(index);
        let width_src = segments.first().map_or(0.0, |s| s.a.core_l);
        let width_mouth = segments.last().map_or(0.0, |s| s.b.core_l);
        rows.push((
            reach.length_l,
            reach.plain,
            reach.sinuosity,
            width_src,
            width_mouth,
            reach.end.as_str(),
        ));
    }
    let lakes = net
        .lakes()
        .lakes()
        .iter()
        .map(|lake| (lake.water_area_l2, lake.level_km))
        .collect::<Vec<_>>();
    reach_report(channels.plate_id(), &rows, &lakes)
}

/// Обход одной сети: собирает [`RiverReport`] из уже построенного графа.
///
/// Отдельно от `collect`, потому что сеть строится на плиту и её нельзя
/// пересчитывать на каждую точку сетки.
pub fn reach_report(
    plate_id: u64,
    reaches: &[(f64, bool, f64, f64, f64, &'static str)],
    lakes: &[(f64, Option<f64>)],
) -> RiverReport {
    let mut out = RiverReport {
        plate_id,
        reaches: reaches.len(),
        ..RiverReport::default()
    };
    let mut sinuosity = Vec::new();
    for &(length_l, plain, sinuosity_value, width_src, width_mouth, end) in reaches {
        out.max_reach_l = out.max_reach_l.max(length_l);
        if plain {
            sinuosity.push(sinuosity_value);
        }
        if length_l > 5.0 && width_src > 0.0 {
            out.long_reaches += 1;
            if width_mouth >= 2.0 * width_src {
                out.wider_mouth += 1;
            }
        }
        match end {
            "sea" | "rift" | "transform" => {
                out.ends_sea += 1;
                out.ends_with_water += 1;
            }
            "lake" => {
                out.ends_lake += 1;
                out.ends_with_water += 1;
            }
            "endorheic" => {
                out.ends_endorheic += 1;
                out.ends_with_water += 1;
            }
            "confluence" => {
                out.ends_confluence += 1;
                out.ends_with_water += 1;
            }
            _ => out.ends_unresolved += 1,
        }
    }
    if !sinuosity.is_empty() {
        sinuosity.sort_by(f64::total_cmp);
        let mid = sinuosity.len() / 2;
        out.plain_sinuosity_median = Some(if sinuosity.len() % 2 == 0 {
            (sinuosity[mid - 1] + sinuosity[mid]) * 0.5
        } else {
            sinuosity[mid]
        });
    }
    for &(area_l2, level_km) in lakes {
        if area_l2 <= 0.0 {
            continue;
        }
        out.lakes += 1;
        out.lake_area_l2 += area_l2;
        // Уровень выше моря (63 блока ≈ 0,063 км): 10.1 просит озёра на 80+.
        if level_km.is_some_and(|level| level > 0.08) {
            out.high_lakes += 1;
        }
    }
    out
}

/// Строка отчёта по рекам и озёрам: те же цели 10.1, что в плане.
pub fn river_rows(report: &RiverReport, rivers: &RiverLake) -> Vec<(&'static str, String, bool)> {
    let mut rows = Vec::new();
    let ends = report.ends_with_water + report.ends_unresolved;
    rows.push((
        "где кончается река",
        if ends == 0 {
            "нет устьев".to_string()
        } else {
            format!("{:.1} % с водой", pct(report.ends_with_water, ends))
        },
        ends > 0 && report.ends_unresolved == 0,
    ));
    rows.push((
        "извилистость равнин",
        report
            .plain_sinuosity_median
            .map(|v| format!("{v:.3}"))
            .unwrap_or_else(|| "—".into()),
        report
            .plain_sinuosity_median
            .is_some_and(|v| (1.2..=2.0).contains(&v)),
    ));
    rows.push((
        "ширина вниз по течению",
        format!("{}/{}", report.wider_mouth, report.long_reaches),
        report.long_reaches > 0 && report.wider_mouth == report.long_reaches,
    ));
    rows.push((
        "длинные реки, L",
        format!("{:.1}", report.max_reach_l),
        report.max_reach_l > 15.0,
    ));
    rows.push((
        "озёра на 100 L²",
        format!("{:.2}", rivers.lakes_per_100_l2()),
        // 10.1: во влажных и холодных низинах ≥ 1 озеро на 100 L². Сетка
        // `stats` не различает «влажную низину», поэтому строка справочная.
        rivers.lakes_per_100_l2() >= 1.0,
    ));
    rows.push((
        "озёра выше моря (R2)",
        format!("{}", report.high_lakes),
        report.high_lakes > 0,
    ));
    rows.push((
        "площадь зеркал, L²",
        format!("{:.2}", report.lake_area_l2),
        report.lake_area_l2 >= 0.0,
    ));
    rows
}

/// Полный отчёт для печати: заголовок и строки.
pub fn text(
    shape: &ClimateShape,
    rivers: &RiverLake,
    report: Option<&RiverReport>,
    diag: Option<&ZoneDiagnostics>,
) -> String {
    let mut out = String::new();
    out.push_str("климат влияет на форму (10.1)\n");
    out.push_str(&format!(
        "  суша без рек/озёр: {} точек; полосы E: {}\n",
        shape.land,
        shape
            .erosion_pct
            .iter()
            .enumerate()
            .map(|(i, v)| format!("E{i} {v:.1}"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    out.push_str("  зона               точек   E0    E1    E2    E3    E4    E5    E6   озёра%  реки%  отличий\n");
    for z in &shape.zones {
        out.push_str(&format!(
            "  {:<17} {:>6}  {:>4.1}  {:>4.1}  {:>4.1}  {:>4.1}  {:>4.1}  {:>4.1}  {:>4.1}  {:>6.2}  {:>5.2}  {:>7}\n",
            z.name,
            z.land,
            z.erosion_pct[0],
            z.erosion_pct[1],
            z.erosion_pct[2],
            z.erosion_pct[3],
            z.erosion_pct[4],
            z.erosion_pct[5],
            z.erosion_pct[6],
            z.lake_pct,
            z.river_pct,
            z.distinct_bands
        ));
    }
    match shape.pass {
        Some(true) => out.push_str("  итог: в цели 10.1\n"),
        Some(false) => out.push_str("  итог: НЕ в цели 10.1\n"),
        None => out.push_str("  итог: нет объёма по зонам\n"),
    }
    for note in &shape.notes {
        out.push_str(&format!("  · {note}\n"));
    }
    if let Some(diag) = diag {
        out.push('\n');
        out.push_str(&diag.text());
    }
    out.push_str(&format!(
        "\nреки и озёра\n  суша {} точек, в русле {} ({:.1} %), вода озёр {} ({:.2} %), на 100 L² {:.2}\n",
        rivers.land,
        rivers.river_land,
        rivers.river_pct(),
        rivers.lake_land,
        rivers.lake_pct(),
        rivers.lakes_per_100_l2()
    ));
    if let Some(report) = report {
        out.push_str(&format!(
            "  сеть плиты {}: цепочек {}, устьев с водой {}, без выхода {}\n",
            report.plate_id, report.reaches, report.ends_with_water, report.ends_unresolved
        ));
        for (name, value, ok) in river_rows(report, rivers) {
            out.push_str(&format!(
                "  {:<24} {:<16} {}\n",
                name,
                value,
                if ok {
                    "в цели"
                } else {
                    "НЕ в цели"
                }
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pct_is_zero_without_total() {
        assert_eq!(pct(0, 0), 0.0);
        assert_eq!(pct(1, 4), 25.0);
    }

    #[test]
    fn e_band_matches_plan_edges() {
        assert_eq!(e_band(-0.9), 0);
        assert_eq!(e_band(-0.78), 1);
        assert_eq!(e_band(0.049), 3);
        assert_eq!(e_band(0.45), 5);
        assert_eq!(e_band(0.9), 6);
    }

    #[test]
    fn reach_report_counts_ends() {
        let reaches = [
            (20.0, true, 1.5, 4.0, 9.0, "sea"),
            (6.0, true, 1.1, 3.0, 3.0, "lake"),
            (2.0, false, 1.0, 2.0, 2.0, "below_threshold"),
            (8.0, true, 1.3, 2.0, 5.0, "endorheic"),
        ];
        let report = reach_report(7, &reaches, &[(12.0, Some(0.3)), (0.0, None)]);
        assert_eq!(report.reaches, 4);
        assert_eq!(report.ends_with_water, 3);
        assert_eq!(report.ends_unresolved, 1);
        assert_eq!(report.ends_sea, 1);
        assert_eq!(report.ends_lake, 1);
        assert_eq!(report.ends_endorheic, 1);
        assert_eq!(report.wider_mouth, 2);
        assert_eq!(report.long_reaches, 3);
        assert_eq!(report.lakes, 1);
        assert_eq!(report.high_lakes, 1);
        assert!(report.plain_sinuosity_median.is_some());
    }

    #[test]
    fn river_rows_mark_failures() {
        let report = RiverReport {
            reaches: 1,
            ends_with_water: 0,
            ends_unresolved: 1,
            max_reach_l: 1.0,
            ..RiverReport::default()
        };
        let rivers = RiverLake {
            area_l2: 1.0,
            ..RiverLake::default()
        };
        let rows = river_rows(&report, &rivers);
        assert!(
            rows.iter()
                .any(|(name, _, ok)| *name == "где кончается река" && !*ok)
        );
        assert!(
            rows.iter()
                .any(|(name, _, ok)| *name == "длинные реки, L" && !*ok)
        );
    }
}

/// Диагностика T123: почему зоны не различаются по полосам E.
///
/// Считает по тем же точкам, что `stats`, но печатает внутренние величины:
/// долю точек, где зону перекрывают горы и плато, распределение `A`
/// (равнина ↔ холмы) и `mountain`, а также ширину зоны (T_bio).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZoneDiagnostics {
    pub points: usize,
    /// Зону перекрыли горы (`m_e > 0.5`).
    pub mountain_override: usize,
    /// Зону перекрыло плато (`plateau > 0.5` и не горы).
    pub plateau_override: usize,
    /// Гистограмма mountain по 5 корзинам.
    pub mountain_hist: [usize; 5],
    /// Доля точек кривого `A`, где зона вообще не влияет.
    pub zone_neutral: usize,
}

impl ZoneDiagnostics {
    pub fn add(&mut self, mountain: f64, plateau: f64, m_e: f64) {
        self.points += 1;
        let bucket = |v: f64| ((v.clamp(0.0, 1.0) * 5.0).floor() as usize).min(4);
        self.mountain_hist[bucket(mountain)] += 1;
        if m_e > 0.5 {
            self.mountain_override += 1;
        } else if plateau * (1.0 - m_e) > 0.5 {
            self.plateau_override += 1;
        }
        if m_e <= 0.01 && plateau <= 0.01 {
            self.zone_neutral += 1;
        }
    }

    pub fn text(&self) -> String {
        if self.points == 0 {
            return "T123 диагностика: нет точек\n".to_string();
        }
        let p = |n: usize| 100.0 * n as f64 / self.points as f64;
        let hist = |h: &[usize; 5]| {
            h.iter()
                .map(|n| format!("{:.0}", p(*n)))
                .collect::<Vec<_>>()
                .join(" / ")
        };
        format!(
            "T123 диагностика зон ({points} точек):\n  \
             горы перекрывают зону {mount:.1} %, плато {plateau:.1} %, зона влияет {neutral:.1} %\n  \
             mountain по корзинам 0…1: {m_hist}\n",
            points = self.points,
            mount = p(self.mountain_override),
            plateau = p(self.plateau_override),
            neutral = p(self.zone_neutral),
            m_hist = hist(&self.mountain_hist),
        )
    }
}
