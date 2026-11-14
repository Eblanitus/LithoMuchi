//! Метрики T100 / раздела 10.1 пересмотра. Отдельная диагностика поверхности,
//! не подмена существующих метрик 12.4 команды `stats` без новых флагов.

use std::collections::{HashMap, HashSet, VecDeque};

use litho_core::vanilla::peaks_valleys;
use litho_core::{Mode, SurfaceKind};

use crate::height::{HeightInput, HeightModel, SEA_LEVEL};
use crate::plan_metrics::{self, ClimateShape, PlanAccumulator, RiverLake, ZoneDiagnostics};
use crate::render::RenderOptions;

const E_EDGES: [f64; 6] = [-0.78, -0.375, -0.2225, 0.05, 0.45, 0.55];
const HYPS_EDGES: [f64; 4] = [63.0, 70.0, 90.0, 130.0];

/// 12.4: зональный профиль осадков. Псевдоширота φ = 0…90° (0 — «экватор»)
/// бьётся на 18 поясов по 5°; в каждом — среднее `p_mm` по всем точкам
/// скана. Осадки атмосферные, поэтому океан из профиля не исключается.
pub const ZONAL_BAND_DEG: f64 = 5.0;
pub const ZONAL_BANDS: usize = 18;
/// Пояс с меньшим числом точек в профиль не берётся: один выброс луча
/// влаги не должен становиться максимумом или минимумом всего пояса.
pub const ZONAL_MIN_SAMPLES: usize = 8;

#[derive(Clone, Copy, Debug, Default)]
pub struct ZonalProfile {
    pub sums: [f64; ZONAL_BANDS],
    pub counts: [usize; ZONAL_BANDS],
}

impl ZonalProfile {
    pub fn add(&mut self, phi_deg: f64, p_mm: f64) {
        if !phi_deg.is_finite() || !p_mm.is_finite() || phi_deg < 0.0 {
            return;
        }
        // φ = 90° ровно (полюс) относится к последнему поясу 85–90°.
        let band = (phi_deg.min(90.0 - 1e-9) / ZONAL_BAND_DEG) as usize;
        self.sums[band] += p_mm;
        self.counts[band] += 1;
    }

    pub fn samples(&self) -> usize {
        self.counts.iter().sum()
    }

    /// Среднее по поясу; `None`, если точек нет или их меньше порога.
    pub fn mean(&self, band: usize) -> Option<f64> {
        (self.counts[band] >= ZONAL_MIN_SAMPLES).then(|| self.sums[band] / self.counts[band] as f64)
    }

    fn arg_extreme<F>(&self, better: F) -> Option<usize>
    where
        F: Fn(f64, f64) -> bool,
    {
        let mut best: Option<(usize, f64)> = None;
        for i in 0..ZONAL_BANDS {
            let Some(v) = self.mean(i) else { continue };
            if best.is_none_or(|(_, b)| better(v, b)) {
                best = Some((i, v));
            }
        }
        best.map(|(i, _)| i)
    }

    pub fn max_band(&self) -> Option<usize> {
        self.arg_extreme(|v, b| v > b)
    }

    pub fn min_band(&self) -> Option<usize> {
        self.arg_extreme(|v, b| v < b)
    }

    /// Второй максимум по 12.4 лежит в 45–55°: пояса 9 и 10.
    pub fn front_band(&self) -> Option<usize> {
        (9..=10)
            .filter_map(|i| self.mean(i).map(|v| (i, v)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// Ориентир 12.4 без придуманных порогов: максимум в 0–5°, минимум
    /// в 20–30° (пояса 4, 5), в 45–55° — локальный второй максимум.
    /// `None` — профиль не покрыт сканом: это «НЕТ ДАННЫХ», а не провал.
    pub fn in_target(&self) -> Option<bool> {
        if self.mean(0).is_none()
            || (self.mean(4).is_none() && self.mean(5).is_none())
            || self.front_band().is_none()
        {
            return None;
        }
        let front = self.front_band()?;
        let front_v = self.mean(front)?;
        let left = front.checked_sub(1).and_then(|i| self.mean(i));
        let right = self.mean((front + 1).min(ZONAL_BANDS - 1));
        let local_max = left.is_none_or(|v| front_v > v) && right.is_none_or(|v| front_v > v);
        Some(self.max_band() == Some(0) && matches!(self.min_band(), Some(4 | 5)) && local_max)
    }

    pub fn verdict_text(&self) -> &'static str {
        match self.in_target() {
            Some(true) => "в ориентире",
            Some(false) => "ВНЕ ОРИЕНТИРА",
            None => "НЕТ ДАННЫХ",
        }
    }

    /// Фрагмент JSON для ключа `climate124` — без обрамляющих фигурных
    /// скобок: их печатает `HeightStats::climate124_json`.
    pub fn json(&self) -> String {
        let values = (0..ZONAL_BANDS)
            .map(|i| {
                self.mean(i)
                    .map_or_else(|| "null".to_string(), |v| format!("{v:.3}"))
            })
            .collect::<Vec<_>>()
            .join(", ");
        let counts = self
            .counts
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let deg = |b: Option<usize>| b.map_or_else(|| "null".to_string(), |i| (i * 5).to_string());
        format!(
            "\"zonal_profile\": {{\"band_deg\": 5, \"points\": {}, \"p_mm\": [{values}], \"samples\": [{counts}], \"max_deg\": {}, \"min_deg\": {}, \"front_deg\": {}, \"in_target\": {}}}",
            self.samples(),
            deg(self.max_band()),
            deg(self.min_band()),
            deg(self.front_band()),
            self.in_target()
                .map_or_else(|| "null".to_string(), |v| v.to_string()),
        )
    }
}

/// 12.4: поперечники плит по основной сетке. Плита учитывается, только
/// если она целиком попала в скан: плиты, касающиеся края области, обрезаны
/// сканом, и их видимая площадь занижает поперечник. Поперечник —
/// эквивалентный по площади: `2·sqrt(A/π)`, площадь — число точек плиты,
/// умноженное на площадь точки сетки в L².
#[derive(Clone, Debug, Default)]
pub struct PlateSizes {
    pub counts: HashMap<u64, usize>,
    pub border: HashSet<u64>,
}

impl PlateSizes {
    pub fn add(&mut self, plate_id: u64, on_border: bool) {
        *self.counts.entry(plate_id).or_insert(0) += 1;
        if on_border {
            self.border.insert(plate_id);
        }
    }

    /// Поперечники плит, целиком попавших в скан, по возрастанию.
    pub fn diameters(&self, point_area_l2: f64) -> Vec<f64> {
        if point_area_l2 <= 0.0 {
            return Vec::new();
        }
        let mut out: Vec<f64> = self
            .counts
            .iter()
            .filter(|(id, _)| !self.border.contains(id))
            .map(|(_, count)| 2.0 * ((*count as f64 * point_area_l2) / std::f64::consts::PI).sqrt())
            .collect();
        out.sort_by(|a, b| a.total_cmp(b));
        out
    }

    pub fn median_diameter(&self, point_area_l2: f64) -> Option<f64> {
        let diameters = self.diameters(point_area_l2);
        let n = diameters.len();
        if n == 0 {
            None
        } else if n % 2 == 1 {
            Some(diameters[n / 2])
        } else {
            Some((diameters[n / 2 - 1] + diameters[n / 2]) * 0.5)
        }
    }

    pub fn min_diameter(&self, point_area_l2: f64) -> Option<f64> {
        self.diameters(point_area_l2).first().copied()
    }

    /// Фрагмент JSON для ключа `climate124`, как у `ZonalProfile::json`.
    pub fn json(&self, point_area_l2: f64) -> String {
        let fmt = |v: Option<f64>| v.map_or_else(|| "null".to_string(), |x| format!("{x:.3}"));
        format!(
            "\"plates\": {{\"plates\": {}, \"counted\": {}, \"median_diameter_l\": {}, \"min_diameter_l\": {}}}",
            self.counts.len(),
            self.diameters(point_area_l2).len(),
            fmt(self.median_diameter(point_area_l2)),
            fmt(self.min_diameter(point_area_l2)),
        )
    }
}

/// 12.4: асимметрия берегов в поясе пассатов 10–25° (Д-079). Береговая
/// точка суши — суша (`C` не ниже порога океаничности) хотя бы с одним
/// соседом-водой по 4-соседству. Восточный берег — вода только с востока
/// (+x), западный — только с запада (−x); точки с водой с обеих сторон
/// или только с севера/юга в сравнение не идут: у них нет однозначной
/// восточности. Крайние строки и столбцы скана не оцениваются — у них
/// нет полного соседства. Строки подаются потоково по три: оценивается
/// средняя, поэтому сетку хранить не нужно.
#[derive(Clone, Debug, Default)]
pub struct CoastAsymmetry {
    rows: VecDeque<Vec<(bool, f64, f64)>>,
    pub east_sum: f64,
    pub east_count: usize,
    pub west_sum: f64,
    pub west_count: usize,
}

impl CoastAsymmetry {
    pub fn push_row(&mut self, row: Vec<(bool, f64, f64)>) {
        self.rows.push_back(row);
        if self.rows.len() < 3 {
            return;
        }
        let north = self.rows.pop_front().unwrap_or_default();
        let (Some(cur), Some(south)) = (self.rows.front(), self.rows.back()) else {
            return;
        };
        let (e_sum, e_count, w_sum, w_count) = Self::score_row(&north, cur, south);
        self.east_sum += e_sum;
        self.east_count += e_count;
        self.west_sum += w_sum;
        self.west_count += w_count;
    }

    fn score_row(
        north: &[(bool, f64, f64)],
        cur: &[(bool, f64, f64)],
        south: &[(bool, f64, f64)],
    ) -> (f64, usize, f64, usize) {
        let mut out = (0.0, 0_usize, 0.0, 0_usize);
        if cur.len() < 3 || north.len() != cur.len() || south.len() != cur.len() {
            return out;
        }
        for i in 1..cur.len() - 1 {
            let (land, phi, p) = cur[i];
            if !land || !p.is_finite() || !(10.0..=25.0).contains(&phi) {
                continue;
            }
            let east_ocean = !cur[i + 1].0;
            let west_ocean = !cur[i - 1].0;
            let coastal = east_ocean || west_ocean || !north[i].0 || !south[i].0;
            if !coastal {
                continue;
            }
            match (east_ocean, west_ocean) {
                (true, false) => {
                    out.0 += p;
                    out.1 += 1;
                }
                (false, true) => {
                    out.2 += p;
                    out.3 += 1;
                }
                _ => {}
            }
        }
        out
    }

    pub fn east_mean(&self) -> Option<f64> {
        (self.east_count > 0).then(|| self.east_sum / self.east_count as f64)
    }

    pub fn west_mean(&self) -> Option<f64> {
        (self.west_count > 0).then(|| self.west_sum / self.west_count as f64)
    }

    pub fn ratio(&self) -> Option<f64> {
        match (self.east_mean(), self.west_mean()) {
            (Some(east), Some(west)) if west > 0.0 => Some(east / west),
            _ => None,
        }
    }

    /// Ориентир 12.4: восточные берега в среднем не суше западных в 2 раза
    /// и больше. `None` — точек мало: «НЕТ ДАННЫХ», а не провал.
    pub fn in_target(&self) -> Option<bool> {
        if self.east_count < ZONAL_MIN_SAMPLES || self.west_count < ZONAL_MIN_SAMPLES {
            return None;
        }
        self.ratio().map(|ratio| ratio >= 2.0)
    }

    /// Фрагмент JSON для ключа `climate124`, как у `ZonalProfile::json`.
    pub fn json(&self) -> String {
        let fmt = |v: Option<f64>| v.map_or_else(|| "null".to_string(), |x| format!("{x:.3}"));
        format!(
            "\"coast_asymmetry\": {{\"band_deg\": [10, 25], \"east_mean_mm\": {}, \"west_mean_mm\": {}, \"ratio\": {}, \"east_samples\": {}, \"west_samples\": {}, \"in_target\": {}}}",
            fmt(self.east_mean()),
            fmt(self.west_mean()),
            fmt(self.ratio()),
            self.east_count,
            self.west_count,
            self.in_target()
                .map_or_else(|| "null".to_string(), |v| v.to_string()),
        )
    }
}

/// 12.4: дождевая тень за хребтами (Д-081). Хребтовая точка — суша с
/// `M > 0,6`. По направлению ветра (`climate.wind` дует по ветру, луч
/// влаги идёт против него) берутся соседние точки сетки: наветренная —
/// шаг против ветра, подветренная — шаг по ветру; направление квантуется
/// к ближайшему из 8 соседей. Шаг основной сетки при обычном замере —
/// 1,25 L, это ближайшая дискретизация «в пределах 1 L» из 12.4.
/// Снижение — по средним: `1 − P_подветр / P_наветр`; обе стороны берутся
/// как есть, включая океан: осадки атмосферные. Край скана не оценивается.
/// Ячейка строки дождевой тени: (суша, осадки мм/год, `M`, ветер x, ветер z).
type ShadowCell = (bool, f64, f64, f64, f64);

#[derive(Clone, Debug, Default)]
pub struct RainShadow {
    rows: VecDeque<Vec<ShadowCell>>,
    pub ridge_count: usize,
    pub windward_sum: f64,
    pub leeward_sum: f64,
}

impl RainShadow {
    pub fn push_row(&mut self, row: Vec<ShadowCell>) {
        self.rows.push_back(row);
        if self.rows.len() < 3 {
            return;
        }
        let north = self.rows.pop_front().unwrap_or_default();
        let (Some(cur), Some(south)) = (self.rows.front(), self.rows.back()) else {
            return;
        };
        let (count, windward, leeward) = Self::score_row(&north, cur, south);
        self.ridge_count += count;
        self.windward_sum += windward;
        self.leeward_sum += leeward;
    }

    fn score_row(
        north: &[ShadowCell],
        cur: &[ShadowCell],
        south: &[ShadowCell],
    ) -> (usize, f64, f64) {
        let mut out = (0_usize, 0.0, 0.0);
        if cur.len() < 3 || north.len() != cur.len() || south.len() != cur.len() {
            return out;
        }
        let rows = [north, cur, south];
        for i in 1..cur.len() - 1 {
            let (land, _, mountain, wind_x, wind_z) = cur[i];
            if !land || mountain <= 0.6 || !wind_x.is_finite() || !wind_z.is_finite() {
                continue;
            }
            let norm = (wind_x * wind_x + wind_z * wind_z).sqrt();
            if norm <= 0.0 {
                continue;
            }
            // Ближайший из 8 соседей к направлению ветра.
            let mut best = (0_i64, 0_i64, f64::NEG_INFINITY);
            for dz in -1_i64..=1 {
                for dx in -1_i64..=1 {
                    if dx == 0 && dz == 0 {
                        continue;
                    }
                    let len = ((dx * dx + dz * dz) as f64).sqrt();
                    let dot = (wind_x * dx as f64 + wind_z * dz as f64) / (norm * len);
                    if dot > best.2 {
                        best = (dx, dz, dot);
                    }
                }
            }
            let (dx, dz) = (best.0, best.1);
            let at = |ox: i64, oz: i64| rows[(oz + 1) as usize][(i as i64 + ox) as usize].1;
            let windward = at(-dx, -dz);
            let leeward = at(dx, dz);
            if !windward.is_finite() || !leeward.is_finite() {
                continue;
            }
            out.0 += 1;
            out.1 += windward;
            out.2 += leeward;
        }
        out
    }

    pub fn windward_mean(&self) -> Option<f64> {
        (self.ridge_count > 0).then(|| self.windward_sum / self.ridge_count as f64)
    }

    pub fn leeward_mean(&self) -> Option<f64> {
        (self.ridge_count > 0).then(|| self.leeward_sum / self.ridge_count as f64)
    }

    /// Снижение осадков за хребтами в процентах от наветренной стороны.
    pub fn drop_pct(&self) -> Option<f64> {
        match (self.windward_mean(), self.leeward_mean()) {
            (Some(windward), Some(leeward)) if windward > 0.0 => {
                Some(100.0 * (1.0 - leeward / windward))
            }
            _ => None,
        }
    }

    /// Ориентир 12.4: снижение не меньше 40 %. `None` — хребтовых точек
    /// мало: «НЕТ ДАННЫХ», а не провал.
    pub fn in_target(&self) -> Option<bool> {
        if self.ridge_count < ZONAL_MIN_SAMPLES {
            return None;
        }
        self.drop_pct().map(|drop| drop >= 40.0)
    }

    /// Фрагмент JSON для ключа `climate124`, как у `ZonalProfile::json`.
    pub fn json(&self) -> String {
        let fmt = |v: Option<f64>| v.map_or_else(|| "null".to_string(), |x| format!("{x:.3}"));
        format!(
            "\"rain_shadow\": {{\"ridge_samples\": {}, \"windward_mean_mm\": {}, \"leeward_mean_mm\": {}, \"drop_pct\": {}, \"in_target\": {}}}",
            self.ridge_count,
            fmt(self.windward_mean()),
            fmt(self.leeward_mean()),
            fmt(self.drop_pct()),
            self.in_target()
                .map_or_else(|| "null".to_string(), |v| v.to_string()),
        )
    }
}

/// 12.4: «островная пыль» (Д-082) — компоненты связности суши на сетке
/// границ (шаг 32 блока, как объём приёмки 10.1 по шагу). Клочок —
/// компонента из 4-соседних клеток суши площадью меньше 0,01 L², целиком
/// внутри пятна: касающиеся края пятна обрезаны сканом и не считаются,
/// как краевые плиты в Д-078. Нормировка — на просканированную площадь
/// всех пятен; вердикт при площади не меньше 10 L², иначе объёма мало и
/// это «НЕТ ДАННЫХ» (на обычном малом объёме границ так и будет).
#[derive(Clone, Debug, Default)]
pub struct IslandDust {
    pub specks: usize,
    pub area_l2: f64,
}

impl IslandDust {
    pub fn add_patch(&mut self, land: &[bool], n: usize, cell_area_l2: f64) {
        if n == 0 || land.len() != n * n || cell_area_l2 <= 0.0 {
            return;
        }
        self.area_l2 += n as f64 * n as f64 * cell_area_l2;
        let mut seen = vec![false; n * n];
        let mut stack: Vec<usize> = Vec::new();
        for start in 0..n * n {
            if !land[start] || seen[start] {
                continue;
            }
            let mut cells = 0_usize;
            let mut touches_border = false;
            stack.clear();
            stack.push(start);
            seen[start] = true;
            while let Some(k) = stack.pop() {
                cells += 1;
                let (i, j) = (k % n, k / n);
                touches_border |= i == 0 || j == 0 || i + 1 == n || j + 1 == n;
                let neighbours = [
                    (i > 0).then(|| k - 1),
                    (i + 1 < n).then(|| k + 1),
                    (j > 0).then(|| k - n),
                    (j + 1 < n).then(|| k + n),
                ];
                for next in neighbours.into_iter().flatten() {
                    if land[next] && !seen[next] {
                        seen[next] = true;
                        stack.push(next);
                    }
                }
            }
            if !touches_border && cells as f64 * cell_area_l2 < 0.01 {
                self.specks += 1;
            }
        }
    }

    /// Клочков на 1000 L² просканированной площади.
    pub fn per_1000_l2(&self) -> Option<f64> {
        (self.area_l2 > 0.0).then(|| self.specks as f64 * 1000.0 / self.area_l2)
    }

    /// Ориентир 12.4: не больше 5 клочков на 1000 L².
    pub fn in_target(&self) -> Option<bool> {
        if self.area_l2 < 10.0 {
            return None;
        }
        self.per_1000_l2().map(|rate| rate <= 5.0)
    }

    /// Фрагмент JSON для ключа `climate124`, как у `ZonalProfile::json`.
    pub fn json(&self) -> String {
        let fmt = |v: Option<f64>| v.map_or_else(|| "null".to_string(), |x| format!("{x:.3}"));
        format!(
            "\"island_dust\": {{\"specks\": {}, \"area_l2\": {}, \"per_1000_l2\": {}, \"in_target\": {}}}",
            self.specks,
            fmt(Some(self.area_l2)),
            fmt(self.per_1000_l2()),
            self.in_target()
                .map_or_else(|| "null".to_string(), |v| v.to_string()),
        )
    }
}

#[derive(Clone, Debug)]
pub struct StatsOptions {
    pub size: usize,
    pub span_l: f64,
    pub relief_size: usize,
    pub relief_step: f64,
    /// T151: мелкие сетки (окна 128 и границы) повторяются в `patches²`
    /// пятнах, разложенных по всей области `span_l`. 1 — одно пятно в центре.
    pub patches: usize,
    pub fast: bool,
    /// T134: отдельная мелкая сетка границ. Шаг обязан делить 32 блока,
    /// иначе скачок «в пределах 32 блоков» не измеряется точно.
    pub boundary_size: usize,
    pub boundary_step: f64,
}
impl StatsOptions {
    pub fn parse(args: &[String], base: &RenderOptions) -> Result<Self, String> {
        let number = |flag: &str, default: f64| -> Result<f64, String> {
            match crate::value(args, flag) {
                Some(text) => text.parse::<f64>().map_err(|e| format!("{flag}: {e}")),
                None => Ok(default),
            }
        };
        let size = number("--size", 64.0)?;
        let relief_size = number("--relief-size", 32.0)?;
        if !size.is_finite()
            || size < 1.0
            || size.fract() != 0.0
            || size > 4096.0
            || !relief_size.is_finite()
            || relief_size < 4.0
            || relief_size.fract() != 0.0
            || relief_size > 1024.0
            || relief_size as usize % 4 != 0
        {
            return Err("--size: 1…4096; --relief-size: 4…1024, кратно 4".into());
        }
        let span_l = number("--span-l", 200.0)?;
        let relief_step = number("--relief-step", 32.0)?;
        if !span_l.is_finite() || span_l <= 0.0 || !relief_step.is_finite() || relief_step <= 0.0 {
            return Err("--span-l и --relief-step должны быть конечными и > 0".into());
        }
        let patches = number("--patches", 1.0)?;
        if !patches.is_finite() || patches < 1.0 || patches.fract() != 0.0 || patches > 32.0 {
            return Err("--patches: целое 1…32".into());
        }
        let boundary_size = number("--boundary-size", 64.0)?;
        let boundary_step = number("--boundary-step", 32.0)?;
        if !boundary_size.is_finite()
            || boundary_size < 4.0
            || boundary_size.fract() != 0.0
            || boundary_size > 8192.0
            || !boundary_step.is_finite()
            || boundary_step <= 0.0
            || boundary_step > 32.0
            || (32.0 / boundary_step).fract() != 0.0
        {
            return Err("--boundary-size: 4…8192; --boundary-step: делитель 32 в (0, 32]".into());
        }
        let fast = match crate::value(args, "--path").as_deref() {
            Some("reference") => false,
            Some("fast") | None => true,
            Some(other) => return Err(format!("--path: неизвестный путь {other}")),
        };
        let _ = base;
        Ok(Self {
            size: size as usize,
            span_l,
            relief_size: relief_size as usize,
            relief_step,
            patches: patches as usize,
            fast,
            boundary_size: boundary_size as usize,
            boundary_step,
        })
    }
}

#[derive(Clone, Debug)]
pub struct HeightStats {
    pub seed: i64,
    pub mode: Mode,
    pub samples: usize,
    pub land: usize,
    /// 12.4: суша по C ≥ порога, включая реки и озёра (как `stats_row`,
    /// только на основной сетке T100). Сухопутная гипсометрия (`land`)
    /// реки и воду исключает, а доли климата 12.4 считаются по всей суше.
    pub land_c: usize,
    /// 12.4 на суше `land_c`: горы M > 0,5; пустыни — провинции К1+К2;
    /// леса — К5+К6 в поясах П2–П6; полярные — пояса П0+П1.
    pub mountains_c: usize,
    pub deserts_c: usize,
    pub forests_c: usize,
    pub polar_c: usize,
    /// 12.4: точка появления хороша по критериям 7.12 (`spawn::evaluate`
    /// один раз на замер — поиск уже сделан в `Generator::new`).
    pub spawn_good: bool,
    pub spawn_shift_l: (f64, f64),
    /// 12.4: профиль осадков по 18 поясам псевдошироты. Считается по всей
    /// основной сетке T100, включая океан, до всех `continue` в цикле.
    pub zonal: ZonalProfile,
    /// 12.4: поперечники плит по той же сетке (все точки, океан включён:
    /// плита не кончается на берегу). `plate_area_l2` — площадь одной
    /// точки сетки в L², нужна, чтобы из счётчиков получить поперечники.
    pub plates: PlateSizes,
    pub plate_area_l2: f64,
    /// 12.4: асимметрия берегов пассатов по той же сетке (Д-079).
    pub coast: CoastAsymmetry,
    /// 12.4: дождевая тень за хребтами по той же сетке (Д-081).
    pub shadow: RainShadow,
    /// 12.4: островная пыль по сетке границ (Д-082).
    pub dust: IslandDust,
    pub river_samples: usize,
    pub river_water: usize,
    pub water_samples: usize,
    pub swamp_samples: usize,
    pub dry_samples: usize,
    pub estuary_samples: usize,
    pub deferred_samples: usize,
    pub gallery_samples: usize,
    pub hyps: [usize; 5],
    pub erosion: [usize; 7],
    pub valley: usize,
    /// Д-057: суша в полосе `river ≥ 0.5` вне выреза русла — плоский сухой
    /// берег с речным W≈0. В срез «долина» не входит (как и до Д-057, когда
    /// вся полоса считалась руслом), но печатается отдельно.
    pub dry_bank: usize,
    pub relief_windows: usize,
    pub relief_median: Option<f64>,
    pub flat_windows: usize,
    pub boundary: BoundaryMetrics,
    /// 10.1 «климат влияет на форму» и строки «Реки и озёра».
    /// `None` — версия не `v1-dev` или объём не посчитан.
    pub plan: Option<PlanMetrics>,
}

/// 10.1: две группы строк, которых не было в замере `final1`.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanMetrics {
    pub shape: ClimateShape,
    pub rivers: RiverLake,
    /// T123: почему зоны не различаются по полосам E.
    pub diag: ZoneDiagnostics,
}
/// T134 / 10.1: метрики границ на отдельной сетке с шагом ≤32 блока.
/// Суша = C≥порога без рек/озёр. Скачок влажности и ветер считаются на парах
/// ровно через 32 блока по обеим осям. Прямолинейность ищет цепочки рёбер
/// границ любых направлений: хорда >512 блоков, отклонение середин <16;
/// диагональная лестница учитывается, ветвления разбивают цепочки.
#[derive(Clone, Debug)]
pub struct BoundaryMetrics {
    pub size: usize,
    pub step: f64,
    pub span_l: f64,
    pub samples: usize,
    pub land: usize,
    pub humidity_jump_pairs: usize,
    pub humidity_pairs: usize,
    pub humidity_jump_land: usize,
    /// T137: климатическая суша без гидрологических переопределений биома.
    pub climate_land: usize,
    pub climate_humidity_pairs: usize,
    pub climate_humidity_jump_pairs: usize,
    pub climate_humidity_jump_land: usize,
    /// T151: пары со скачком влажности по причине (см. [`JUMP_CAUSES`]).
    pub jump_cause_pairs: [usize; JUMP_CAUSES.len()],
    /// T151: скачок по полю H, которое уходит в игру (вся суша): класс —
    /// ванильная полоса влажности числа `vanilla.h`, а не метка биома.
    pub field_humidity_jump_land: usize,
    pub field_humidity_jump_pairs: usize,
    pub wind_max_deg: f64,
    pub wind_pairs: usize,
    pub boundary_edges: usize,
    pub straight_edges: usize,
    pub thin_land: usize,
    pub min_strip_blocks: Option<f64>,
}
/// T151: чем гидрология переопределила биом в узле. Порядок — приоритет при
/// наложении; `climate` — обычная климатическая суша T137.
pub const JUMP_CAUSES: [&str; 7] = [
    "climate",
    "river",
    "gallery",
    "swamp",
    "dry_basin",
    "estuary",
    "other",
];

/// Границы ванильных полос влажности 26.3 (`OverworldBiomeBuilder`).
const VANILLA_H_EDGES: [f64; 4] = [-0.35, -0.1, 0.1, 0.3];

/// Номер ванильной полосы влажности 0…4 для числа H, уходящего в игру.
pub fn vanilla_humidity_band(h: f64) -> u8 {
    VANILLA_H_EDGES.partition_point(|edge| h >= *edge) as u8
}

impl BoundaryMetrics {
    pub fn field_humidity_jump_pct(&self) -> f64 {
        pct(self.field_humidity_jump_land, self.land)
    }
    /// Скачок ≥2 полос между узлами в 32 блоках по полю H на всей суше.
    /// Вызывается после `analyze_grid` (знаменатель — её `land`).
    fn count_field_jumps(&mut self, band: &[u8], land: &[bool]) {
        let n = self.size;
        let offset = (32.0 / self.step).round() as usize;
        let mut jump = vec![false; n * n];
        for j in 0..n {
            for i in 0..n {
                let k = j * n + i;
                if !land[k] {
                    continue;
                }
                for (di, dj) in [(offset, 0_usize), (0, offset)] {
                    if i + di >= n || j + dj >= n {
                        continue;
                    }
                    let q = (j + dj) * n + i + di;
                    if land[q] && band[k].abs_diff(band[q]) >= 2 {
                        self.field_humidity_jump_pairs += 1;
                        jump[k] = true;
                        jump[q] = true;
                    }
                }
            }
        }
        self.field_humidity_jump_land += jump.iter().filter(|&&v| v).count();
    }
    /// Раскладывает пары со скачком влажности (те же, что в `analyze_grid`)
    /// по причине: старшая из двух гидрологических меток пары.
    fn count_jump_causes(&mut self, class: &[u8], land: &[bool], feature: &[u8]) {
        let n = self.size;
        let offset = (32.0 / self.step).round() as usize;
        for j in 0..n {
            for i in 0..n {
                let k = j * n + i;
                if !land[k] {
                    continue;
                }
                for (di, dj) in [(offset, 0_usize), (0, offset)] {
                    if i + di >= n || j + dj >= n {
                        continue;
                    }
                    let q = (j + dj) * n + i + di;
                    if land[q] && (class[k] % 5).abs_diff(class[q] % 5) >= 2 {
                        self.jump_cause_pairs[feature[k].max(feature[q]) as usize] += 1;
                    }
                }
            }
        }
    }

    pub(crate) fn empty(size: usize, step: f64, l_blocks: f64) -> Self {
        Self {
            size,
            step,
            span_l: size as f64 * step / l_blocks,
            samples: size * size,
            land: 0,
            humidity_jump_pairs: 0,
            humidity_pairs: 0,
            humidity_jump_land: 0,
            climate_land: 0,
            climate_humidity_pairs: 0,
            climate_humidity_jump_pairs: 0,
            climate_humidity_jump_land: 0,
            jump_cause_pairs: [0; JUMP_CAUSES.len()],
            field_humidity_jump_land: 0,
            field_humidity_jump_pairs: 0,
            wind_max_deg: 0.0,
            wind_pairs: 0,
            boundary_edges: 0,
            straight_edges: 0,
            thin_land: 0,
            min_strip_blocks: None,
        }
    }
    pub fn humidity_jump_pct(&self) -> f64 {
        pct(self.humidity_jump_land, self.land)
    }
    pub fn humidity_pair_pct(&self) -> f64 {
        pct(self.humidity_jump_pairs, self.humidity_pairs)
    }
    pub fn climate_humidity_jump_pct(&self) -> f64 {
        pct(self.climate_humidity_jump_land, self.climate_land)
    }
    pub fn climate_humidity_pair_pct(&self) -> f64 {
        pct(
            self.climate_humidity_jump_pairs,
            self.climate_humidity_pairs,
        )
    }
    pub fn straight_pct(&self) -> f64 {
        pct(self.straight_edges, self.boundary_edges)
    }
    pub fn thin_pct(&self) -> f64 {
        pct(self.thin_land, self.land)
    }
    pub fn acceptance_volume(&self) -> bool {
        self.span_l >= 200.0 && self.step <= 32.0
    }
    fn analyze_grid(&mut self, class: &[u8], land: &[bool], climate: &[bool], jump: &mut [bool]) {
        let n = self.size;
        let offset = (32.0 / self.step).round() as usize;
        let at = |i: usize, j: usize| j * n + i;
        let mut climate_jump = vec![false; n * n];
        for j in 0..n {
            for i in 0..n {
                let k = at(i, j);
                if !land[k] {
                    continue;
                }
                self.land += 1;
                if climate[k] {
                    self.climate_land += 1;
                }
                for (di, dj) in [(offset, 0_usize), (0, offset)] {
                    let ii = i + di;
                    let jj = j + dj;
                    if ii >= n || jj >= n {
                        continue;
                    }
                    let q = at(ii, jj);
                    if !land[q] {
                        continue;
                    }
                    self.humidity_pairs += 1;
                    let climate_pair = climate[k] && climate[q];
                    if climate_pair {
                        self.climate_humidity_pairs += 1;
                    }
                    let h0 = class[k] % 5;
                    let h1 = class[q] % 5;
                    if h0.abs_diff(h1) >= 2 {
                        self.humidity_jump_pairs += 1;
                        jump[k] = true;
                        jump[q] = true;
                        if climate_pair {
                            self.climate_humidity_jump_pairs += 1;
                            climate_jump[k] = true;
                            climate_jump[q] = true;
                        }
                    }
                }
            }
        }
        // `+=`: при `--patches` сетка разбирается по пятну за вызов.
        self.humidity_jump_land += jump.iter().filter(|&&v| v).count();
        self.climate_humidity_jump_land += climate_jump.iter().filter(|&&v| v).count();
        // T134: границы любых направлений, включая диагональную лестницу.
        // Знаменатель — длина сеточных рёбер; числитель — рёбра на длинной
        // хорде одного класса с отклонением середин <16 блоков.
        let (edges, straight) = straight_boundary_edges(class, land, n, self.step);
        self.boundary_edges += edges;
        self.straight_edges += straight;
        // Узкие полоски: прогон одного класса между другими сухопутными
        // классами уже 32 блоков хотя бы по одной оси.
        let mut thin = vec![false; n * n];
        for j in 0..n {
            let mut start = 0;
            while start < n {
                let k = at(start, j);
                if !land[k] {
                    start += 1;
                    continue;
                }
                let mut end = start + 1;
                while end < n && land[at(end, j)] && class[at(end, j)] == class[k] {
                    end += 1;
                }
                let width = (end - start) as f64 * self.step;
                let bounded = start > 0 && end < n && land[at(start - 1, j)] && land[at(end, j)];
                if bounded {
                    self.min_strip_blocks =
                        Some(self.min_strip_blocks.map_or(width, |m| m.min(width)));
                    if width < 32.0 {
                        for q in start..end {
                            thin[at(q, j)] = true;
                        }
                    }
                }
                start = end;
            }
        }
        for i in 0..n {
            let mut start = 0;
            while start < n {
                let k = at(i, start);
                if !land[k] {
                    start += 1;
                    continue;
                }
                let mut end = start + 1;
                while end < n && land[at(i, end)] && class[at(i, end)] == class[k] {
                    end += 1;
                }
                let width = (end - start) as f64 * self.step;
                let bounded = start > 0 && end < n && land[at(i, start - 1)] && land[at(i, end)];
                if bounded {
                    self.min_strip_blocks =
                        Some(self.min_strip_blocks.map_or(width, |m| m.min(width)));
                    if width < 32.0 {
                        for q in start..end {
                            thin[at(i, q)] = true;
                        }
                    }
                }
                start = end;
            }
        }
        self.thin_land += thin.iter().filter(|&&v| v).count();
    }
}

/// T134: ищем длинные прямые участки границ биомов, а не только осевые.
///
/// Граница представлена рёбрами между соседними сухопутными клетками разного
/// класса. Рёбра одного неупорядоченного класса соединяются через общие углы
/// клеток в цепочки; ветвления разбивают цепочку. Для цепочки (и скользящего
/// окна внутри неё) берём хорду между серединами крайних рёбер: длина >512
/// блоков и максимальное отклонение середин всех рёбер <16 блоков. Это ловит
/// диагональную лестницу и произвольный наклон, но остаётся консервативным на
/// ветвлениях. Порядок обхода фиксирован порядком рёбер, не итерацией HashMap.
fn straight_boundary_edges(class: &[u8], land: &[bool], n: usize, step: f64) -> (usize, usize) {
    use std::collections::HashMap;

    #[derive(Clone, Copy)]
    struct Edge {
        a: (u32, u32),
        b: (u32, u32),
        mid: (f64, f64),
        pair: u16,
    }
    let at = |i: usize, j: usize| j * n + i;
    let pair = |a: usize, b: usize| -> u16 {
        u16::from(class[a].min(class[b])) * 25 + u16::from(class[a].max(class[b]))
    };
    let mut edges = Vec::new();
    let mut adj: HashMap<(u32, u32, u16), Vec<usize>> = HashMap::new();
    let push = |a: (u32, u32),
                b: (u32, u32),
                mid: (f64, f64),
                p: u16,
                edges: &mut Vec<Edge>,
                adj: &mut HashMap<(u32, u32, u16), Vec<usize>>| {
        let id = edges.len();
        edges.push(Edge { a, b, mid, pair: p });
        adj.entry((a.0, a.1, p)).or_default().push(id);
        adj.entry((b.0, b.1, p)).or_default().push(id);
    };
    for j in 0..n {
        for i in 0..n.saturating_sub(1) {
            let a = at(i, j);
            let b = at(i + 1, j);
            if land[a] && land[b] && class[a] != class[b] {
                push(
                    ((i + 1) as u32, j as u32),
                    ((i + 1) as u32, (j + 1) as u32),
                    (i as f64 + 1.0, j as f64 + 0.5),
                    pair(a, b),
                    &mut edges,
                    &mut adj,
                );
            }
        }
    }
    for i in 0..n {
        for j in 0..n.saturating_sub(1) {
            let a = at(i, j);
            let b = at(i, j + 1);
            if land[a] && land[b] && class[a] != class[b] {
                push(
                    (i as u32, (j + 1) as u32),
                    ((i + 1) as u32, (j + 1) as u32),
                    (i as f64 + 0.5, j as f64 + 1.0),
                    pair(a, b),
                    &mut edges,
                    &mut adj,
                );
            }
        }
    }
    let total = edges.len();
    if total == 0 {
        return (0, 0);
    }
    let degree = |v: (u32, u32), p: u16| adj.get(&(v.0, v.1, p)).map_or(0, Vec::len);
    let mut visited = vec![false; total];
    let mut chains: Vec<Vec<usize>> = Vec::new();
    let walk =
        |start: usize, from: (u32, u32), visited: &mut [bool], chains: &mut Vec<Vec<usize>>| {
            let mut chain = Vec::new();
            let mut id = start;
            let mut v = from;
            loop {
                visited[id] = true;
                chain.push(id);
                let e = edges[id];
                let next_v = if e.a == v { e.b } else { e.a };
                if degree(next_v, e.pair) != 2 {
                    break;
                }
                let Some(list) = adj.get(&(next_v.0, next_v.1, e.pair)) else {
                    break;
                };
                let Some(&next) = list.iter().find(|&&k| !visited[k]) else {
                    break;
                };
                id = next;
                v = next_v;
            }
            chains.push(chain);
        };
    // Сначала цепочки с концами/ветвлениями, затем замкнутые контуры.
    for id in 0..total {
        if visited[id] {
            continue;
        }
        let e = edges[id];
        let from = if degree(e.a, e.pair) != 2 {
            Some(e.a)
        } else if degree(e.b, e.pair) != 2 {
            Some(e.b)
        } else {
            None
        };
        if let Some(v) = from {
            walk(id, v, &mut visited, &mut chains);
        }
    }
    for id in 0..total {
        if !visited[id] {
            walk(id, edges[id].a, &mut visited, &mut chains);
        }
    }
    let mut straight = vec![false; total];
    let mark = |chain: &[usize], straight: &mut [bool]| -> bool {
        if chain.len() < 2 {
            return false;
        }
        let a = edges[chain[0]].mid;
        let b = edges[chain[chain.len() - 1]].mid;
        let dx = b.0 - a.0;
        let dz = b.1 - a.1;
        let chord_grid = (dx * dx + dz * dz).sqrt();
        if chord_grid * step <= 512.0 {
            return false;
        }
        for &id in chain {
            let m = edges[id].mid;
            let dev = ((m.0 - a.0) * dz - (m.1 - a.1) * dx).abs() * step / chord_grid;
            if dev >= 16.0 {
                return false;
            }
        }
        for &id in chain {
            straight[id] = true;
        }
        true
    };
    // Окно заведомо длиннее 512 блоков даже для худшей диагональной лестницы.
    let window = ((512.0 * std::f64::consts::SQRT_2 / step).ceil() as usize).saturating_add(2);
    for chain in &chains {
        if mark(chain, &mut straight) {
            continue;
        }
        if chain.len() > window {
            for start in 0..=chain.len() - window {
                mark(&chain[start..start + window], &mut straight);
            }
        }
    }
    (total, straight.iter().filter(|&&v| v).count())
}

fn pct(count: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        100.0 * count as f64 / total as f64
    }
}

impl HeightStats {
    /// T151: пустой носитель для тестов вердикта 10.1 без полного скана.
    /// Все счётчики нулевые, сетка границ — заданного размера.
    #[cfg(test)]
    pub fn empty_for_tests(size: usize, step: usize) -> Self {
        Self {
            seed: 0,
            mode: Mode::Gameplay,
            samples: 0,
            land: 0,
            land_c: 0,
            mountains_c: 0,
            deserts_c: 0,
            forests_c: 0,
            polar_c: 0,
            spawn_good: false,
            spawn_shift_l: (0.0, 0.0),
            zonal: ZonalProfile::default(),
            plates: PlateSizes::default(),
            plate_area_l2: 0.0,
            coast: CoastAsymmetry::default(),
            shadow: RainShadow::default(),
            dust: IslandDust::default(),
            river_samples: 0,
            river_water: 0,
            water_samples: 0,
            swamp_samples: 0,
            dry_samples: 0,
            estuary_samples: 0,
            deferred_samples: 0,
            gallery_samples: 0,
            hyps: [0; 5],
            erosion: [0; 7],
            valley: 0,
            dry_bank: 0,
            relief_windows: 0,
            relief_median: None,
            flat_windows: 0,
            boundary: BoundaryMetrics::empty(size, step as f64, 250.0),
            plan: None,
        }
    }
    pub fn collect(
        base: &RenderOptions,
        opts: &StatsOptions,
        model: &HeightModel,
    ) -> Result<Self, String> {
        let generator = crate::tune::generator(base.seed, base.mode, base.version)?;
        let step = opts.span_l * base.mode.l_blocks() / opts.size as f64;
        let mut out = Self {
            seed: base.seed,
            mode: base.mode,
            samples: opts.size * opts.size,
            land: 0,
            land_c: 0,
            mountains_c: 0,
            deserts_c: 0,
            forests_c: 0,
            polar_c: 0,
            spawn_good: false,
            spawn_shift_l: (0.0, 0.0),
            zonal: ZonalProfile::default(),
            plates: PlateSizes::default(),
            plate_area_l2: 0.0,
            coast: CoastAsymmetry::default(),
            shadow: RainShadow::default(),
            dust: IslandDust::default(),
            river_samples: 0,
            river_water: 0,
            water_samples: 0,
            swamp_samples: 0,
            dry_samples: 0,
            estuary_samples: 0,
            deferred_samples: 0,
            gallery_samples: 0,
            hyps: [0; 5],
            erosion: [0; 7],
            valley: 0,
            dry_bank: 0,
            relief_windows: 0,
            relief_median: None,
            flat_windows: 0,
            boundary: BoundaryMetrics::empty(
                opts.boundary_size,
                opts.boundary_step,
                base.mode.l_blocks(),
            ),
            plan: None,
        };
        out.plate_area_l2 = (opts.span_l / opts.size as f64).powi(2);
        let land_threshold = generator.params().climate.ocean_c_threshold;
        // 12.4: точка появления. Поиск уже выполнен в `Generator::new`,
        // здесь одна проверка критериев 7.12 на эффективных параметрах
        // (с `--set` — на изменённых, как и весь замер).
        let shift = generator.spawn_shift();
        out.spawn_good =
            litho_core::spawn::evaluate(shift, generator.seed(), generator.params()).good;
        out.spawn_shift_l = (shift.x, shift.z);
        // 10.1: «климат влияет на форму» и «Реки и озёра» считаются на тех же
        // точках, что гипсометрия, без второго `world_sample` на точку.
        let plan_dev = base.version.is_water_relief_dev();
        let mut plan = PlanAccumulator::new();
        for j in 0..opts.size {
            let mut coast_row: Vec<(bool, f64, f64)> = Vec::with_capacity(opts.size);
            let mut shadow_row: Vec<(bool, f64, f64, f64, f64)> = Vec::with_capacity(opts.size);
            for i in 0..opts.size {
                let x = base.center_x + (i as f64 - opts.size as f64 * 0.5) * step;
                let z = base.center_z + (j as f64 - opts.size as f64 * 0.5) * step;
                let world = if opts.fast {
                    generator.world_sample_fast(x, z)
                } else {
                    generator.world_sample(x, z)
                }
                .ok_or_else(|| format!("нет образца в ({x}, {z})"))?;
                let v = world.vanilla;
                // 12.4: зональный профиль осадков. Осадки атмосферные,
                // океан не исключается, поэтому точка берётся до всех
                // `continue` ниже (вода, русло, не-суша).
                out.zonal
                    .add(world.climate.pole.phi_deg, world.climate.precip.p_mm);
                // 12.4: поперечники плит. Отдельный plate_sample на точку:
                // WorldSample владельца плиты не отдаёт.
                if let Some(plate) = generator.plate_sample(x, z) {
                    let on_border = i == 0 || j == 0 || i + 1 == opts.size || j + 1 == opts.size;
                    out.plates.add(plate.plate_id(), on_border);
                }
                let input = HeightInput::from_fields(v.c, v.e, v.w);
                let input = if base.pv {
                    input.with_pv(world.pv)
                } else {
                    input
                };
                let input = if model.is_owned() {
                    input.with_owned_y(world.height.surface_y)
                } else {
                    input
                };
                let y = model.surface_y(input)?;
                let c_land = v.c >= land_threshold;
                // 12.4 (Д-079): строка для асимметрии берегов собирается
                // до всех `continue` ниже, как зональный профиль.
                coast_row.push((
                    c_land,
                    world.climate.pole.phi_deg,
                    world.climate.precip.p_mm,
                ));
                shadow_row.push((
                    c_land,
                    world.climate.precip.p_mm,
                    world.climate.surface.mountain,
                    world.climate.wind.x,
                    world.climate.wind.z,
                ));
                // 12.4: доли климата на всей суше, включая реки и озёра.
                // Зона климатическая и под руслами (Д-066 касается только
                // ванильной ячейки), отдельного прохода не нужно.
                if c_land {
                    out.land_c += 1;
                    if world.climate.surface.mountain > 0.5 {
                        out.mountains_c += 1;
                    }
                    let zone = world.class.zone;
                    if zone.province <= 2 {
                        out.deserts_c += 1;
                    }
                    if zone.province >= 5 && (2..=6).contains(&zone.belt) {
                        out.forests_c += 1;
                    }
                    if zone.belt <= 1 {
                        out.polar_c += 1;
                    }
                }
                // Д-057: в v1-dev «точка русла» — узкий вырез самого русла
                // (`channels.bank`), где и должна стоять вода. Широкое поле
                // `river` сглаживает C/E/W вокруг и водой быть не обязано.
                let river = if base.version.is_water_relief_dev() {
                    world.hydrology.channels.bank >= 0.5
                } else {
                    world.river >= 0.5
                };
                if c_land && river {
                    out.river_samples += 1;
                    if y < SEA_LEVEL {
                        out.river_water += 1;
                    }
                }
                let h = world.hydrology;
                if plan_dev {
                    // Морфозона — та же, что задаёт форму рельефа (T122): не
                    // метка биома, у которой у рек служебный класс (Д-066).
                    let dominant = generator
                        .morpho_weights(x, z)
                        .map(|w| w.dominant())
                        .unwrap_or(usize::MAX);
                    // Котловина без воды: отложенная до R2 (T143/T145) и сухая
                    // по климату различаются — иначе «озёр в зоне 0 %» читается
                    // как «озёр тут не бывает», хотя это разные вещи.
                    let basin = if h.r2_deferred > 0.5 {
                        plan_metrics::Basin::Deferred
                    } else if h.dry_basin > 0.5 {
                        plan_metrics::Basin::Dry
                    } else {
                        plan_metrics::Basin::None
                    };
                    plan.add(c_land, v.e, river, h.water > 0.5, basin, dominant);
                    // T123: те же величины, что входят в перевод E, чтобы
                    // видеть перекрытие гор/плато, а не догадываться о нём.
                    let params = generator.params();
                    let surface = world.climate.surface;
                    let m_e = plan_metrics::mountain_weight(surface.mountain, params);
                    plan.diagnose(surface.mountain, surface.plateau, m_e);
                }
                out.water_samples += usize::from(h.water > 0.5);
                out.swamp_samples += usize::from(h.swamp > 0.5);
                out.dry_samples += usize::from(h.dry_basin > 0.5);
                out.estuary_samples += usize::from(h.estuary > 0.5);
                out.deferred_samples += usize::from(h.r2_deferred > 0.5);
                out.gallery_samples += usize::from(h.gallery >= 0.5 && h.gallery > h.river);
                // T115: вода R1 исключена из сухопутной гипсометрии; высокое
                // озеро осталось суша/болото и честно учтено deferred_samples.
                if !c_land || river || h.water > 0.5 {
                    continue;
                }
                out.land += 1;
                out.hyps[HYPS_EDGES.partition_point(|edge| y >= *edge)] += 1;
                out.erosion[E_EDGES.partition_point(|edge| v.e >= *edge)] += 1;
                let pv = if base.pv {
                    world.pv
                } else {
                    peaks_valleys(v.w)
                };
                if base.version.is_water_relief_dev() && world.river >= 0.5 {
                    out.dry_bank += 1;
                } else if pv < -0.85 {
                    out.valley += 1;
                }
            }
            out.coast.push_row(coast_row);
            out.shadow.push_row(shadow_row);
        }
        // Окна 128×128 при шаге 32 считаются отдельной мелкой сеткой: грубая
        // сетка 200 L не должна изображать мелкие холмы средним значением.
        // T151: центры пятен. Одно пятно — центр области, как раньше; иначе
        // равномерная сетка `patches²` центров по всей области `span_l`.
        let span_blocks = opts.span_l * base.mode.l_blocks();
        let mut centers = Vec::with_capacity(opts.patches * opts.patches);
        for pj in 0..opts.patches {
            for pi in 0..opts.patches {
                if opts.patches == 1 {
                    centers.push((base.center_x, base.center_z));
                } else {
                    let at =
                        |p: usize| ((p as f64 + 0.5) / opts.patches as f64 - 0.5) * span_blocks;
                    centers.push((
                        (base.center_x + at(pi)).floor(),
                        (base.center_z + at(pj)).floor(),
                    ));
                }
            }
        }
        let n = opts.relief_size;
        let mut ranges = Vec::new();
        for &(center_x, center_z) in &centers {
            let mut heights = vec![0.0; n * n];
            let mut land = vec![false; n * n];
            for j in 0..n {
                for i in 0..n {
                    let x = center_x + (i as f64 - n as f64 * 0.5) * opts.relief_step;
                    let z = center_z + (j as f64 - n as f64 * 0.5) * opts.relief_step;
                    let world = if opts.fast {
                        generator.world_sample_fast(x, z)
                    } else {
                        generator.world_sample(x, z)
                    }
                    .ok_or_else(|| format!("нет образца рельефа в ({x}, {z})"))?;
                    let v = world.vanilla;
                    let input = HeightInput::from_fields(v.c, v.e, v.w);
                    let input = if base.pv {
                        input.with_pv(world.pv)
                    } else {
                        input
                    };
                    let input = if model.is_owned() {
                        input.with_owned_y(world.height.surface_y)
                    } else {
                        input
                    };
                    heights[j * n + i] = model.surface_y(input)?;
                    land[j * n + i] = v.c >= land_threshold && world.river < 0.5;
                }
            }
            for j in (0..n).step_by(4) {
                for i in (0..n).step_by(4) {
                    let indices = (0..4).flat_map(|dz| (0..4).map(move |dx| (j + dz) * n + i + dx));
                    let mut min = f64::INFINITY;
                    let mut max = f64::NEG_INFINITY;
                    let mut all_land = true;
                    for k in indices {
                        all_land &= land[k];
                        min = min.min(heights[k]);
                        max = max.max(heights[k]);
                    }
                    if !all_land {
                        continue;
                    }
                    let range = max - min;
                    out.relief_windows += 1;
                    if range < 4.0 {
                        out.flat_windows += 1;
                    }
                    ranges.push(range);
                }
            }
        }
        ranges.sort_by(f64::total_cmp);
        if !ranges.is_empty() {
            let mid = ranges.len() / 2;
            out.relief_median = Some(if ranges.len() % 2 == 0 {
                (ranges[mid - 1] + ranges[mid]) * 0.5
            } else {
                ranges[mid]
            });
        }
        // T134: отдельная сетка границ. Ветер сравнивается потоково на парах
        // через 32 блока, чтобы не хранить два f64 на каждый узел большой карты.
        let bn = opts.boundary_size;
        let bstep = opts.boundary_step;
        let boff = (32.0 / bstep).round() as usize;
        out.boundary.samples = bn * bn * centers.len();
        for &(center_x, center_z) in &centers {
            let mut bclass = vec![u8::MAX; bn * bn];
            let mut bland = vec![false; bn * bn];
            let mut bclimate = vec![false; bn * bn];
            let mut bfeature = vec![0_u8; bn * bn];
            let mut bfield = vec![0_u8; bn * bn];
            let mut bjump = vec![false; bn * bn];
            let mut wind_rows: Vec<Vec<(f64, f64)>> = vec![vec![(0.0, 0.0); bn]; boff + 1];
            for j in 0..bn {
                let row = j % (boff + 1);
                for i in 0..bn {
                    let x = center_x + (i as f64 - bn as f64 * 0.5) * bstep;
                    let z = center_z + (j as f64 - bn as f64 * 0.5) * bstep;
                    let world = if opts.fast {
                        generator.world_sample_fast(x, z)
                    } else {
                        generator.world_sample(x, z)
                    }
                    .ok_or_else(|| format!("нет образца границы в ({x}, {z})"))?;
                    let v = world.vanilla;
                    let h = world.hydrology;
                    let is_land = v.c >= land_threshold
                        && world.river < 0.5
                        && h.water < 0.5
                        && h.r2_deferred < 0.5;
                    let k = j * bn + i;
                    bland[k] = is_land;
                    // T137: исходная суша T134 остаётся в отчёте; отдельно считаем
                    // климатическую сушу без русел/болот/галерей/котловин/эстуариев.
                    bclimate[k] = is_land
                        && h.kind == SurfaceKind::Land
                        && h.gallery == 0.0
                        && h.swamp == 0.0
                        && h.dry_basin == 0.0
                        && h.estuary == 0.0;
                    bfield[k] = vanilla_humidity_band(v.h);
                    bfeature[k] = if !is_land || bclimate[k] {
                        0
                    } else if h.estuary != 0.0 {
                        5
                    } else if h.dry_basin != 0.0 {
                        4
                    } else if h.swamp != 0.0 {
                        3
                    } else if h.gallery != 0.0 {
                        2
                    } else if h.kind == SurfaceKind::River {
                        1
                    } else {
                        6
                    };
                    bclass[k] = world.class.cell.t * 5 + world.class.cell.h;
                    let w = world.climate.wind;
                    let wl = w.len();
                    let dir = if wl > 1e-12 {
                        (w.x / wl, w.z / wl)
                    } else {
                        (0.0, 0.0)
                    };
                    wind_rows[row][i] = dir;
                    if i >= boff {
                        let prev = wind_rows[row][i - boff];
                        if prev != (0.0, 0.0) && dir != (0.0, 0.0) {
                            let dot = (prev.0 * dir.0 + prev.1 * dir.1).clamp(-1.0, 1.0);
                            let deg = dot.acos() * 180.0 / std::f64::consts::PI;
                            out.boundary.wind_max_deg = out.boundary.wind_max_deg.max(deg);
                            out.boundary.wind_pairs += 1;
                        }
                    }
                    if j >= boff {
                        let prev = wind_rows[(j - boff) % (boff + 1)][i];
                        if prev != (0.0, 0.0) && dir != (0.0, 0.0) {
                            let dot = (prev.0 * dir.0 + prev.1 * dir.1).clamp(-1.0, 1.0);
                            let deg = dot.acos() * 180.0 / std::f64::consts::PI;
                            out.boundary.wind_max_deg = out.boundary.wind_max_deg.max(deg);
                            out.boundary.wind_pairs += 1;
                        }
                    }
                }
            }
            out.boundary
                .analyze_grid(&bclass, &bland, &bclimate, &mut bjump);
            out.boundary.count_jump_causes(&bclass, &bland, &bfeature);
            out.boundary.count_field_jumps(&bfield, &bland);
            // 12.4 (Д-082): пыль — компоненты связности той же маски суши.
            out.dust
                .add_patch(&bland, bn, (bstep / base.mode.l_blocks()).powi(2));
        }
        // 10.1: у v0 нет ни морфозон, ни сети — строка остаётся `null`.
        if plan_dev {
            let area_l2 = opts.span_l * opts.span_l;
            let (shape, rivers, diag) = plan.finish(area_l2);
            out.plan = Some(PlanMetrics {
                shape,
                rivers,
                diag,
            });
        }
        Ok(out)
    }
    fn pct(&self, count: usize, total: usize) -> f64 {
        if total == 0 {
            0.0
        } else {
            100.0 * count as f64 / total as f64
        }
    }
    pub fn flooded(&self) -> f64 {
        self.pct(self.hyps[0], self.land)
    }
    /// 12.4: суша — от всех точек сетки, остальное — от суши `land_c`.
    pub fn land_c_pct(&self) -> f64 {
        self.pct(self.land_c, self.samples)
    }
    pub fn mountains_c_pct(&self) -> f64 {
        self.pct(self.mountains_c, self.land_c)
    }
    pub fn deserts_c_pct(&self) -> f64 {
        self.pct(self.deserts_c, self.land_c)
    }
    pub fn forests_c_pct(&self) -> f64 {
        self.pct(self.forests_c, self.land_c)
    }
    pub fn polar_c_pct(&self) -> f64 {
        self.pct(self.polar_c, self.land_c)
    }
    pub fn e4(&self) -> f64 {
        self.pct(self.erosion[4], self.land)
    }
    pub fn e23(&self) -> f64 {
        self.pct(self.erosion[2] + self.erosion[3], self.land)
    }
    pub fn e01(&self) -> f64 {
        self.pct(self.erosion[0] + self.erosion[1], self.land)
    }
    pub fn e56(&self) -> f64 {
        self.pct(self.erosion[5] + self.erosion[6], self.land)
    }
    pub fn valley_pct(&self) -> f64 {
        self.pct(self.valley, self.land - self.dry_bank)
    }

    pub fn dry_bank_pct(&self) -> f64 {
        self.pct(self.dry_bank, self.land)
    }
    pub fn flat_pct(&self) -> f64 {
        self.pct(self.flat_windows, self.relief_windows)
    }
    pub fn river_water_pct(&self) -> f64 {
        self.pct(self.river_water, self.river_samples)
    }
    pub fn compact(&self) -> String {
        format!(
            "{:>10} {:<10} {:7.2} {:7.2} {:7.2} {:7.2} {:7.2} {:9.2} {:7.2} {:6}/{} {:7.2} {:7.2} {:7.2} {:8.3} {:7.2}",
            self.seed,
            self.mode.as_str(),
            self.flooded(),
            self.e4(),
            self.e23(),
            self.e01(),
            self.e56(),
            self.relief_median.unwrap_or(f64::NAN),
            self.valley_pct(),
            self.relief_windows,
            self.samples,
            self.boundary.humidity_jump_pct(),
            self.boundary.climate_humidity_jump_pct(),
            self.boundary.straight_pct(),
            self.boundary.wind_max_deg,
            self.boundary.thin_pct()
        )
    }
    pub fn text(&self, base: &RenderOptions, opts: &StatsOptions, model: &HeightModel) -> String {
        let mut out = format!(
            "T100: сид={} режим={} версия={} путь={}\n\
             высота: {}\n\
             {height_note}\n\
             область={span}×{span} L, шаг={step:.3} блока, точек={samples}, суша без русел={land}\n\
             затопленная суша y<63 {flood:6.2}% (быстрая цель ≤10%, v1 ≤5%)\n\
             гипсометрия <63 / 63–70 / 70–90 / 90–130 / ≥130: {h0:.2} / {h1:.2} / {h2:.2} / {h3:.2} / {h4:.2}%\n\
             E4 {e4:.2}% (быстрая цель ≤60%); E2+E3 {e23:.2}%; E0+E1 {e01:.2}%; E5+E6 {e56:.2}%\n\
             долина PV<-0.85 {valley:.2}% (v1 ≤8% gameplay / ≤6% scientific)\n\
             окна {window}×{window} блоков: медиана перепада {median:.2} (быстрая цель ≥8), <4 блоков {flat:.2}%, окон={windows}\n\
             вода в руслах r≥0.5: {water:.2}% из {river} точек (R1: поверхность ниже y=63); сухой берег в полосе русла {dry_bank:.2}% суши\n\
             T115–T116 от всех точек: вода {surface_water:.2}%, болото {swamp:.2}%, сухая котловина {dry:.2}%, эстуарий {estuary:.2}%, отложено R2 {deferred:.2}%, галерейный лес {gallery:.2}%\n\
             12.4 климат на суше C≥порога ({land_c} точек): суша {land_c_pct:.2}% (25–40), горы {mnt_c:.2}% (5–20), пустыни {dst_c:.2}% (15–30), леса {frs_c:.2}% (30–50), полярные {pol_c:.2}% (10–25)\n\
             12.4 точка появления: {spawn} (сдвиг {shift_x:.2}, {shift_z:.2} L)\n",
            base.seed,
            base.mode.as_str(),
            base.version.as_str(),
            if opts.fast { "fast" } else { "reference" },
            model.source.label(),
            height_note = if model.is_owned() {
                "T144: собственная высота ядра (offset/factor/jaggedness); ванильные сплайны не используются"
            } else {
                "WARNING: приближение 128*(1+offset); factor/jaggedness/3D-шум не учтены"
            },
            span = opts.span_l,
            step = opts.span_l * base.mode.l_blocks() / opts.size as f64,
            samples = self.samples,
            land = self.land,
            flood = self.flooded(),
            h0 = self.flooded(),
            h1 = self.pct(self.hyps[1], self.land),
            h2 = self.pct(self.hyps[2], self.land),
            h3 = self.pct(self.hyps[3], self.land),
            h4 = self.pct(self.hyps[4], self.land),
            e4 = self.e4(),
            e23 = self.e23(),
            e01 = self.e01(),
            e56 = self.e56(),
            valley = self.valley_pct(),
            window = 4.0 * opts.relief_step,
            median = self.relief_median.unwrap_or(f64::NAN),
            flat = self.flat_pct(),
            windows = self.relief_windows,
            water = self.river_water_pct(),
            river = self.river_samples,
            dry_bank = self.dry_bank_pct(),
            surface_water = self.pct(self.water_samples, self.samples),
            swamp = self.pct(self.swamp_samples, self.samples),
            dry = self.pct(self.dry_samples, self.samples),
            estuary = self.pct(self.estuary_samples, self.samples),
            deferred = self.pct(self.deferred_samples, self.samples),
            gallery = self.pct(self.gallery_samples, self.samples),
            land_c = self.land_c,
            land_c_pct = self.land_c_pct(),
            mnt_c = self.mountains_c_pct(),
            dst_c = self.deserts_c_pct(),
            frs_c = self.forests_c_pct(),
            pol_c = self.polar_c_pct(),
            spawn = if self.spawn_good {
                "хороша"
            } else {
                "НЕ ГОДНА"
            },
            shift_x = self.spawn_shift_l.0,
            shift_z = self.spawn_shift_l.1,
        );
        let fmt_band = |v: Option<f64>| v.map_or_else(|| "нет".to_string(), |x| format!("{x:.0}"));
        let fmt_deg = |b: Option<usize>| {
            b.map_or_else(
                || "нет".to_string(),
                |i| format!("{}–{}°", i * 5, (i + 1) * 5),
            )
        };
        out.push_str(&format!(
            "12.4 зональный профиль P(φ), мм/год: 0–5° {p0}, 20–30° {p20}/{p25}, 45–55° {p45}/{p50}; макс {max}, мин {min}, второй макс {front} — {verdict}\n",
            p0 = fmt_band(self.zonal.mean(0)),
            p20 = fmt_band(self.zonal.mean(4)),
            p25 = fmt_band(self.zonal.mean(5)),
            p45 = fmt_band(self.zonal.mean(9)),
            p50 = fmt_band(self.zonal.mean(10)),
            max = fmt_deg(self.zonal.max_band()),
            min = fmt_deg(self.zonal.min_band()),
            front = fmt_deg(self.zonal.front_band()),
            verdict = self.zonal.verdict_text(),
        ));
        let fmt_l = |v: Option<f64>| v.map_or_else(|| "нет".to_string(), |x| format!("{x:.1}"));
        out.push_str(&format!(
            "12.4 плиты: медиана поперечника {median} L (ориентир 12–25), наименьшая {min} L (ориентир ≥ 4), плит в скане {all}, целиком {counted}\n",
            median = fmt_l(self.plates.median_diameter(self.plate_area_l2)),
            min = fmt_l(self.plates.min_diameter(self.plate_area_l2)),
            all = self.plates.counts.len(),
            counted = self.plates.diameters(self.plate_area_l2).len(),
        ));
        let fmt_mm = |v: Option<f64>| v.map_or_else(|| "нет".to_string(), |x| format!("{x:.0}"));
        let fmt_ratio = |v: Option<f64>| v.map_or_else(|| "нет".to_string(), |x| format!("{x:.2}"));
        out.push_str(&format!(
            "12.4 берега пассатов 10–25°: восток {east} мм/год ({en} точек), запад {west} мм/год ({wn} точек), отношение {ratio} — {verdict}\n",
            east = fmt_mm(self.coast.east_mean()),
            en = self.coast.east_count,
            west = fmt_mm(self.coast.west_mean()),
            wn = self.coast.west_count,
            ratio = fmt_ratio(self.coast.ratio()),
            verdict = match self.coast.in_target() {
                Some(true) => "в ориентире",
                Some(false) => "ВНЕ ОРИЕНТИРА",
                None => "НЕТ ДАННЫХ",
            },
        ));
        let fmt_pct = |v: Option<f64>| v.map_or_else(|| "нет".to_string(), |x| format!("{x:.1}"));
        out.push_str(&format!(
            "12.4 дождевая тень (M>0.6): наветренная {windward} мм/год, подветренная {leeward} мм/год, снижение {drop}% по {n} хребтовым точкам — {verdict}\n",
            windward = fmt_mm(self.shadow.windward_mean()),
            leeward = fmt_mm(self.shadow.leeward_mean()),
            drop = fmt_pct(self.shadow.drop_pct()),
            n = self.shadow.ridge_count,
            verdict = match self.shadow.in_target() {
                Some(true) => "в ориентире",
                Some(false) => "ВНЕ ОРИЕНТИРА",
                None => "НЕТ ДАННЫХ",
            },
        ));
        out.push_str(&format!(
            "12.4 островная пыль: {specks} клочков (<0.01 L²) на {area:.2} L² сетки границ, {rate} на 1000 L² (ориентир ≤ 5) — {verdict}\n",
            specks = self.dust.specks,
            area = self.dust.area_l2,
            rate = fmt_ratio(self.dust.per_1000_l2()),
            verdict = match self.dust.in_target() {
                Some(true) => "в ориентире",
                Some(false) => "ВНЕ ОРИЕНТИРА",
                None => "НЕТ ДАННЫХ",
            },
        ));
        let b = &self.boundary;
        out.push_str(&format!(
            "T134 границы: сетка {bn}×{bn} ({bsamples} точек), шаг {bs:.3} блока, область {span:.3}×{span:.3} L, суша {bl}\n\
             скачок h≥2 всей суши {jump:.2}% точек ({jp}/{pairs} пар, {pair_pct:.2}% пар), ветер max {wind:.3}°/32 блока ({wp} пар)\n\
             климатическая суша {cl}: скачок h≥2 {cjump:.2}% точек ({cjp}/{cpairs} пар, {cpair_pct:.2}% пар); гидро-override исключён\n\
             прямолинейность границ (хорда >512, отклонение <16, любые направления) {straight:.2}% ({se}/{be} рёбер), полоски <32 блоков {thin:.2}% суши, min {minstrip}\n\
             объём приёмки 10.1 (≥200×200 L, шаг≤32): {accept}\n",
            bn = b.size,
            bsamples = b.samples,
            bs = b.step,
            span = b.span_l,
            bl = b.land,
            jump = b.humidity_jump_pct(),
            jp = b.humidity_jump_pairs,
            pairs = b.humidity_pairs,
            pair_pct = b.humidity_pair_pct(),
            cl = b.climate_land,
            cjump = b.climate_humidity_jump_pct(),
            cjp = b.climate_humidity_jump_pairs,
            cpairs = b.climate_humidity_pairs,
            cpair_pct = b.climate_humidity_pair_pct(),
            wind = b.wind_max_deg,
            wp = b.wind_pairs,
            straight = b.straight_pct(),
            se = b.straight_edges,
            be = b.boundary_edges,
            thin = b.thin_pct(),
            minstrip = b.min_strip_blocks.map_or("нет".to_string(), |v| format!("{v:.1}")),
            accept = if b.acceptance_volume() { "да" } else { "НЕТ (малый скан)" },
        ));
        if model.is_synthetic() {
            out.push_str(
                "WARNING: синтетический сплайн, НЕ ваниль. Для приёмки нужен --game-data.\n",
            );
        }
        if self.land == 0 || self.relief_windows == 0 {
            out.push_str("WARNING: нет суши/полных сухопутных окон; проценты и медиана не являются оценкой рельефа.\n");
        }
        if let Some(plan) = &self.plan {
            out.push('\n');
            out.push_str(&plan_metrics::text(
                &plan.shape,
                &plan.rivers,
                None,
                Some(&plan.diag),
            ));
        }
        out.push_str("T115–T116/T141: сеть/низкие озёра подключены к C/E/W и биомам (R1), собственный PV доступен через --pv. Высокие озёра и вода выше моря — T143/T145. Проценты воды здесь не заменяют проверку реальной высоты.\n");
        out
    }
    pub fn json(&self, base: &RenderOptions, opts: &StatsOptions, model: &HeightModel) -> String {
        let causes = JUMP_CAUSES
            .iter()
            .zip(self.boundary.jump_cause_pairs)
            .map(|(name, count)| format!("\"{name}\": {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{{\n  \"seed\": {}, \"mode\": {}, \"version\": {}, \"path\": {},\n  \"height_source\": {}, \"height_data_hash\": \"{:016x}\", \"synthetic\": {}, \"pv_field\": {},\n  \"approximation\": {},\n  \"span_l\": {}, \"size\": {}, \"samples\": {}, \"land_samples\": {},\n  \"flooded_land_pct\": {}, \"hyps_pct\": {:?}, \"erosion_pct\": {:?},\n  \"valley_pct\": {}, \"set\": {}, \"params_hash\": \"{:016x}\", \"patches\": {}, \"relief_step\": {}, \"relief_windows\": {}, \"relief_median\": {}, \"flat_windows_pct\": {},\n  \"river_samples\": {}, \"river_water_pct\": {}, \"gallery_samples\": {}, \"gallery_pct\": {},\n  \"boundary\": {{\"size\": {}, \"step\": {}, \"span_l\": {}, \"land\": {}, \"humidity_jump_pct\": {}, \"humidity_jump_pair_pct\": {}, \"humidity_jump_pairs\": {}, \"humidity_pairs\": {}, \"climate_land\": {}, \"climate_humidity_jump_pct\": {}, \"climate_humidity_pair_pct\": {}, \"climate_humidity_jump_pairs\": {}, \"climate_humidity_pairs\": {}, \"humidity_jump_cause_pairs\": {{{}}}, \"field_humidity_jump_pct\": {}, \"field_humidity_jump_pairs\": {}, \"wind_max_deg\": {}, \"wind_pairs\": {}, \"straight_pct\": {}, \"straight_edges\": {}, \"boundary_edges\": {}, \"straight_model\": \"grid-edge-chord-512-deviation-16\", \"thin_pct\": {}, \"min_strip_blocks\": {}, \"acceptance_volume\": {}}},\n  \"climate124\": {},\n  \"plan\": {}\n}}\n",
            self.seed,
            crate::json::quote(self.mode.as_str()),
            crate::json::quote(base.version.as_str()),
            crate::json::quote(if opts.fast { "fast" } else { "reference" }),
            crate::json::quote(&model.source.label()),
            model.data_hash,
            model.is_synthetic(),
            base.pv,
            crate::json::quote(if model.is_owned() {
                "litho-core owned I2 surface; no vanilla splines"
            } else {
                "128*(1+offset); no factor/jaggedness/3D noise"
            }),
            opts.span_l,
            opts.size,
            self.samples,
            self.land,
            self.flooded(),
            self.hyps.map(|n| self.pct(n, self.land)),
            self.erosion.map(|n| self.pct(n, self.land)),
            self.valley_pct(),
            crate::json::quote(&crate::tune::label()),
            crate::tune::params(self.mode, base.version)
                .map(|p| p.params_hash())
                .unwrap_or(0),
            opts.patches,
            opts.relief_step,
            self.relief_windows,
            self.relief_median
                .map_or("null".to_string(), |v| v.to_string()),
            self.flat_pct(),
            self.river_samples,
            self.river_water_pct(),
            self.gallery_samples,
            self.pct(self.gallery_samples, self.samples),
            self.boundary.size,
            self.boundary.step,
            self.boundary.span_l,
            self.boundary.land,
            self.boundary.humidity_jump_pct(),
            self.boundary.humidity_pair_pct(),
            self.boundary.humidity_jump_pairs,
            self.boundary.humidity_pairs,
            self.boundary.climate_land,
            self.boundary.climate_humidity_jump_pct(),
            self.boundary.climate_humidity_pair_pct(),
            self.boundary.climate_humidity_jump_pairs,
            self.boundary.climate_humidity_pairs,
            causes,
            self.boundary.field_humidity_jump_pct(),
            self.boundary.field_humidity_jump_pairs,
            self.boundary.wind_max_deg,
            self.boundary.wind_pairs,
            self.boundary.straight_pct(),
            self.boundary.straight_edges,
            self.boundary.boundary_edges,
            self.boundary.thin_pct(),
            self.boundary
                .min_strip_blocks
                .map_or("null".to_string(), |v| v.to_string()),
            self.boundary.acceptance_volume(),
            self.climate124_json(),
            self.plan_json(),
        )
    }

    /// 12.4: доли климата и точка появления отдельным ключом.
    /// Есть у обеих версий: нужные поля (`mountain`, зона) есть и в v0.
    pub fn climate124_json(&self) -> String {
        let zonal_and_plates = format!(
            "{}, {}, {}, {}, {}",
            self.zonal.json(),
            self.plates.json(self.plate_area_l2),
            self.coast.json(),
            self.shadow.json(),
            self.dust.json()
        );
        format!(
            "{{\"land\": {}, \"land_pct\": {:.4}, \"mountains_pct\": {:.4}, \"deserts_pct\": {:.4}, \"forests_pct\": {:.4}, \"polar_pct\": {:.4}, \"spawn_good\": {}, \"spawn_shift_l\": [{:.4}, {:.4}], {}}}",
            self.land_c,
            self.land_c_pct(),
            self.mountains_c_pct(),
            self.deserts_c_pct(),
            self.forests_c_pct(),
            self.polar_c_pct(),
            self.spawn_good,
            self.spawn_shift_l.0,
            self.spawn_shift_l.1,
            zonal_and_plates
        )
    }

    /// 10.1: «климат влияет на форму» и «Реки и озёра» отдельным ключом.
    /// `null`, если версия не `v1-dev`: у v0 этих сущностей нет.
    pub fn plan_json(&self) -> String {
        let Some(plan) = &self.plan else {
            return "null".to_string();
        };
        let shape = &plan.shape;
        let zones = shape
            .zones
            .iter()
            .map(|z| {
                format!(
                    "{{\"name\": {}, \"land\": {}, \"erosion_pct\": {:?}, \"lake_pct\": {:.4}, \"river_pct\": {:.4}, \"deferred_pct\": {:.4}, \"dry_pct\": {:.4}, \"distinct_bands\": {}, \"max_gap_pp\": {:.3}}}",
                    crate::json::quote(z.name),
                    z.land,
                    z.erosion_pct,
                    z.lake_pct,
                    z.river_pct,
                    z.deferred_pct,
                    z.dry_pct,
                    z.distinct_bands,
                    z.max_gap_pp
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let notes = shape
            .notes
            .iter()
            .map(|n| crate::json::quote(n))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{{\"climate_shape\": {{\"land\": {}, \"erosion_pct\": {:?}, \"lake_pct\": {:.4}, \"river_pct\": {:.4}, \"zones\": [{}], \"pass\": {}, \"notes\": [{}]}}, \"rivers_lakes\": {{\"land\": {}, \"lake_land\": {}, \"river_land\": {}, \"dry_land\": {}, \"lake_pct\": {:.4}, \"river_pct\": {:.4}, \"lakes_per_100_l2\": {:.4}, \"area_l2\": {:.3}}}}}",
            shape.land,
            shape.erosion_pct,
            shape.lake_pct,
            shape.river_pct,
            zones,
            shape.pass.map_or("null".to_string(), |p| p.to_string()),
            notes,
            plan.rivers.land,
            plan.rivers.lake_land,
            plan.rivers.river_land,
            plan.rivers.dry_land,
            plan.rivers.lake_pct(),
            plan.rivers.river_pct(),
            plan.rivers.lakes_per_100_l2(),
            plan.rivers.area_l2
        )
    }
}

pub const COMPACT_HEADER: &str = "      сид режим        flood%     E4%    E23%    E01%    E56%   relief128 valley% windows/samples  hjump%  cj% straight%   wind°   thin%";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::height::HeightSource;
    use litho_core::{GenVersion, Mode};
    #[test]
    fn tiny_stats_are_finite_and_json_is_valid() {
        let base = RenderOptions {
            seed: 42,
            mode: Mode::Gameplay,
            version: GenVersion::V1Dev,
            center_x: 0.0,
            center_z: 0.0,
            size: 8,
            bpp: 32.0,
            path_fast: true,
            pv: false,
        };
        let opts = StatsOptions {
            size: 8,
            span_l: 8.0,
            relief_size: 8,
            relief_step: 32.0,
            patches: 1,
            fast: true,
            boundary_size: 8,
            boundary_step: 32.0,
        };
        let model = HeightModel::load(HeightSource::Synthetic).unwrap();
        let stats = HeightStats::collect(&base, &opts, &model).unwrap();
        assert_eq!(stats.samples, 64);
        assert!(stats.flooded().is_finite());
        let value = crate::json::parse(&stats.json(&base, &opts, &model)).unwrap();
        assert_eq!(value.get("synthetic"), Some(&crate::json::Json::Bool(true)));
        // 12.4: суша с реками/озёрами не меньше сухой, доли конечны,
        // точка появления сида 42 годна (как в ядерном тесте на 100 сидов).
        assert!(stats.land_c >= stats.land);
        assert!(stats.land_c_pct().is_finite());
        assert!(stats.mountains_c_pct().is_finite());
        assert!(stats.deserts_c_pct().is_finite());
        assert!(stats.forests_c_pct().is_finite());
        assert!(stats.polar_c_pct().is_finite());
        assert!(stats.spawn_good, "точка появления сида 42 годна");
        let climate = value.get("climate124").expect("12.4 в отчёте");
        assert_eq!(
            climate.get("spawn_good"),
            Some(&crate::json::Json::Bool(true))
        );
        let mut pv_base = base.clone();
        pv_base.pv = true;
        let pv_stats = HeightStats::collect(&pv_base, &opts, &model).unwrap();
        let pv_value = crate::json::parse(&pv_stats.json(&pv_base, &opts, &model)).unwrap();
        assert_eq!(
            pv_value.get("pv_field"),
            Some(&crate::json::Json::Bool(true))
        );
    }

    #[test]
    fn v1_dev_owned_stats_use_core_height_without_synthetic_warning() {
        let base = RenderOptions {
            seed: 42,
            mode: Mode::Gameplay,
            version: GenVersion::V1Dev,
            center_x: 0.0,
            center_z: 0.0,
            size: 8,
            bpp: 32.0,
            path_fast: true,
            pv: true,
        };
        let opts = StatsOptions {
            size: 8,
            span_l: 8.0,
            relief_size: 8,
            relief_step: 32.0,
            patches: 1,
            fast: true,
            boundary_size: 8,
            boundary_step: 32.0,
        };
        let model = HeightModel::load(HeightSource::Owned).unwrap();
        let stats = HeightStats::collect(&base, &opts, &model).unwrap();
        let value = crate::json::parse(&stats.json(&base, &opts, &model)).unwrap();
        assert_eq!(
            value.get("synthetic"),
            Some(&crate::json::Json::Bool(false))
        );
        assert!(stats.flooded().is_finite());
        assert!(stats.text(&base, &opts, &model).contains("T144"));
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;

    #[test]
    fn axis_metrics_detect_jump_straight_run_and_thin_strip() {
        let n = 40;
        let step = 16.0;
        let mut class = vec![0_u8; n * n];
        let mut land = vec![true; n * n];
        // Вертикальная граница h0/h2 длиннее 512 блоков и узкая полоска h1
        // шириной 16 блоков между ними; скачок h≥2 через 32 блока есть.
        for j in 0..n {
            for i in 0..n {
                class[j * n + i] = if i < 20 {
                    0
                } else if i == 20 {
                    1
                } else {
                    2
                };
            }
        }
        land[0] = false;
        let mut m = BoundaryMetrics::empty(n, step, 250.0);
        let mut jump = vec![false; n * n];
        let climate = land.clone();
        m.analyze_grid(&class, &land, &climate, &mut jump);
        assert!(m.humidity_jump_pairs > 0);
        assert!(m.humidity_jump_pct() > 0.0);
        assert!(m.boundary_edges > 0);
        assert!(
            m.straight_pct() > 90.0,
            "осевая граница должна быть прямой: {}",
            m.straight_pct()
        );
        assert!(m.thin_pct() > 0.0);
        assert_eq!(m.min_strip_blocks, Some(16.0));
        assert!(!m.acceptance_volume());
    }

    #[test]
    fn vanilla_humidity_bands_match_26_3_edges_and_core_centers() {
        let p = litho_core::Params::defaults(litho_core::GenVersion::V1Dev, Mode::Gameplay);
        for (band, center) in p.climate.h_centers.iter().enumerate() {
            assert_eq!(vanilla_humidity_band(*center) as usize, band);
        }
        assert_eq!(vanilla_humidity_band(-0.35), 1);
        assert_eq!(vanilla_humidity_band(0.2999), 3);
        assert_eq!(vanilla_humidity_band(1.0), 4);
    }

    #[test]
    fn field_jump_ignores_biome_labels_and_counts_real_h_steps() {
        let mut m = BoundaryMetrics::empty(3, 32.0, 250.0);
        let land = vec![true; 9];
        m.land = 9;
        // Средний столбец: поле H на две полосы суше соседей.
        let band = [3, 1, 3, 3, 1, 3, 3, 1, 3];
        m.count_field_jumps(&band, &land);
        assert_eq!(m.field_humidity_jump_pairs, 6);
        assert_eq!(m.field_humidity_jump_land, 9);
        let mut flat = BoundaryMetrics::empty(3, 32.0, 250.0);
        flat.count_field_jumps(&[2; 9], &land);
        assert_eq!(flat.field_humidity_jump_pairs, 0);
    }

    #[test]
    fn hydrological_override_is_kept_in_all_land_but_not_in_climate_metric() {
        let n = 8;
        let mut class = vec![0_u8; n * n];
        let land = vec![true; n * n];
        let mut climate = vec![true; n * n];
        // Гидрологический биом с h4 среди климатического h0 даёт скачок всей
        // суши, но не должен записываться в климатическую метрику T137.
        class[4 * n + 4] = 4;
        climate[4 * n + 4] = false;
        let mut m = BoundaryMetrics::empty(n, 32.0, 250.0);
        let mut jump = vec![false; n * n];
        m.analyze_grid(&class, &land, &climate, &mut jump);
        assert!(m.humidity_jump_pairs > 0);
        assert!(m.humidity_jump_pct() > 0.0);
        assert_eq!(m.climate_land, n * n - 1);
        assert_eq!(m.climate_humidity_jump_pairs, 0);
        assert_eq!(m.climate_humidity_jump_pct(), 0.0);
    }

    #[test]
    fn no_land_means_zero_percentages_not_nan() {
        let n = 8;
        let class = vec![0_u8; n * n];
        let land = vec![false; n * n];
        let mut m = BoundaryMetrics::empty(n, 32.0, 250.0);
        let mut jump = vec![false; n * n];
        let climate = land.clone();
        m.analyze_grid(&class, &land, &climate, &mut jump);
        assert_eq!(m.humidity_jump_pct(), 0.0);
        assert_eq!(m.straight_pct(), 0.0);
        assert_eq!(m.thin_pct(), 0.0);
        assert!(m.min_strip_blocks.is_none());
    }

    fn diagonal_grid(n: usize, shift: impl Fn(usize) -> isize) -> Vec<u8> {
        let mut class = vec![0_u8; n * n];
        for j in 0..n {
            for i in 0..n {
                class[j * n + i] = if (i as isize) < j as isize + shift(j) {
                    0
                } else {
                    2
                };
            }
        }
        class
    }

    #[test]
    fn diagonal_staircase_longer_than_512_blocks_is_straight() {
        let n = 40;
        let class = diagonal_grid(n, |_| 0);
        let land = vec![true; n * n];
        let (edges, straight) = straight_boundary_edges(&class, &land, n, 16.0);
        assert!(edges > 0);
        assert_eq!(
            straight, edges,
            "диагональная лестница должна считаться прямой"
        );
    }

    #[test]
    fn short_diagonal_is_not_a_long_straight_boundary() {
        let n = 20;
        let class = diagonal_grid(n, |_| 0);
        let land = vec![true; n * n];
        let (edges, straight) = straight_boundary_edges(&class, &land, n, 16.0);
        assert!(edges > 0);
        assert_eq!(straight, 0);
    }

    #[test]
    fn wavy_diagonal_beyond_sixteen_blocks_is_not_straight() {
        let n = 40;
        // Ступени смещения ±3 клетки = ±48 блоков: больше допуска 16.
        let class = diagonal_grid(n, |j| if (j / 6) % 2 == 0 { 3 } else { -3 });
        let land = vec![true; n * n];
        let (edges, straight) = straight_boundary_edges(&class, &land, n, 16.0);
        assert!(edges > 0);
        assert_eq!(straight, 0);
    }

    #[test]
    fn straight_edges_are_a_union_not_double_counted_at_junctions() {
        let n = 80;
        let mut class = vec![0_u8; n * n];
        // Две длинные границы разных классов пересекаются; ветвление делит
        // цепочки, но каждая половина всё равно длиннее 512 блоков. Рёбра не
        // должны дать числитель больше знаменателя.
        for j in 0..n {
            for i in 0..n {
                class[j * n + i] = if i < 40 {
                    0
                } else if j < 40 {
                    1
                } else {
                    2
                };
            }
        }
        let land = vec![true; n * n];
        let (edges, straight) = straight_boundary_edges(&class, &land, n, 16.0);
        assert!(straight <= edges);
        assert!(straight > 0);
    }
}

#[cfg(test)]
mod zonal_tests {
    use super::*;

    /// Профиль «по учебнику» 12.4: экваториальный максимум, субтропический
    /// минимум, фронтальный второй максимум в 45–55°.
    const TEXTBOOK: [f64; ZONAL_BANDS] = [
        1800.0, 1700.0, 1400.0, 1000.0, 500.0, 520.0, 700.0, 850.0, 1000.0, 1200.0, 1150.0, 900.0,
        800.0, 750.0, 700.0, 680.0, 660.0, 650.0,
    ];

    fn profile(values: &[f64; ZONAL_BANDS]) -> ZonalProfile {
        let mut z = ZonalProfile::default();
        for (i, &p) in values.iter().enumerate() {
            for _ in 0..ZONAL_MIN_SAMPLES {
                z.add(i as f64 * ZONAL_BAND_DEG + 1.0, p);
            }
        }
        z
    }

    #[test]
    fn textbook_profile_is_within_12_4_orientation() {
        let z = profile(&TEXTBOOK);
        assert_eq!(z.samples(), ZONAL_BANDS * ZONAL_MIN_SAMPLES);
        assert_eq!(z.max_band(), Some(0));
        assert_eq!(z.min_band(), Some(4));
        assert_eq!(z.front_band(), Some(9));
        assert_eq!(z.in_target(), Some(true));
        assert_eq!(z.verdict_text(), "в ориентире");
    }

    #[test]
    fn wrong_max_or_min_is_outside_orientation() {
        let mut moved = TEXTBOOK;
        moved[0] = 400.0; // максимум уехал из 0–5°
        let z = profile(&moved);
        assert_eq!(z.in_target(), Some(false));
        assert_eq!(z.verdict_text(), "ВНЕ ОРИЕНТИРА");

        let mut flat = TEXTBOOK;
        flat[16] = 100.0; // глобальный минимум не в 20–30°
        assert_eq!(profile(&flat).in_target(), Some(false));
    }

    #[test]
    fn sparse_bands_and_non_finite_values_are_no_data() {
        let z = ZonalProfile::default();
        assert_eq!(z.in_target(), None);
        assert_eq!(z.verdict_text(), "НЕТ ДАННЫХ");
        assert_eq!(z.samples(), 0);

        let mut sparse = ZonalProfile::default();
        sparse.add(2.0, 99_999.0); // один выброс не должен стать максимумом
        sparse.add(f64::NAN, 1000.0);
        sparse.add(2.0, f64::INFINITY);
        sparse.add(-1.0, 1000.0);
        assert_eq!(sparse.samples(), 1);
        assert_eq!(sparse.max_band(), None);
        assert_eq!(sparse.in_target(), None);
    }

    #[test]
    fn pole_boundary_belongs_to_last_band() {
        let mut z = ZonalProfile::default();
        z.add(90.0, 700.0);
        assert_eq!(z.counts[ZONAL_BANDS - 1], 1);
        assert_eq!(z.counts[0], 0);
    }

    #[test]
    fn zonal_json_is_parseable_and_no_data_is_null() {
        let empty = ZonalProfile::default();
        let value = crate::json::parse(&format!("{{{}}}", empty.json()))
            .expect("зональный профиль — валидный JSON");
        let z = value.get("zonal_profile").expect("ключ zonal_profile");
        assert_eq!(z.get("in_target"), Some(&crate::json::Json::Null));

        let value = crate::json::parse(&format!("{{{}}}", profile(&TEXTBOOK).json()))
            .expect("зональный профиль — валидный JSON");
        assert_eq!(
            value.get("zonal_profile").unwrap().get("in_target"),
            Some(&crate::json::Json::Bool(true))
        );
    }
}

#[cfg(test)]
mod plate_tests {
    use super::*;

    fn sizes() -> PlateSizes {
        let mut p = PlateSizes::default();
        // Плита 1: 100 точек целиком в скане; плита 2: 25 точек целиком;
        // плита 3: 400 точек, но касается края и в поперечники не идёт.
        for _ in 0..100 {
            p.add(1, false);
        }
        for _ in 0..25 {
            p.add(2, false);
        }
        for _ in 0..400 {
            p.add(3, true);
        }
        p
    }

    #[test]
    fn border_plates_are_excluded_from_diameters() {
        let p = sizes();
        assert_eq!(p.counts.len(), 3);
        // Площадь точки 1 L²: поперечники 2·sqrt(100/π) и 2·sqrt(25/π).
        let d = p.diameters(1.0);
        assert_eq!(d.len(), 2);
        assert!((d[0] - 5.6419).abs() < 0.001);
        assert!((d[1] - 11.2838).abs() < 0.001);
        assert!((p.min_diameter(1.0).unwrap() - 5.6419).abs() < 0.001);
        assert!((p.median_diameter(1.0).unwrap() - 8.4629).abs() < 0.001);
    }

    #[test]
    fn median_of_single_plate_and_empty_is_none() {
        let mut p = PlateSizes::default();
        assert_eq!(p.median_diameter(1.0), None);
        assert_eq!(p.min_diameter(1.0), None);
        assert!(p.diameters(0.0).is_empty());
        p.add(7, false);
        // Одна точка площадью 4 L²: 2·sqrt(4/π).
        assert!((p.median_diameter(4.0).unwrap() - 2.2568).abs() < 0.001);
    }

    #[test]
    fn plate_json_is_parseable() {
        let value = crate::json::parse(&format!("{{{}}}", sizes().json(1.0)))
            .expect("плиты — валидный JSON");
        let plates = value.get("plates").expect("ключ plates");
        assert_eq!(plates.get("plates"), Some(&crate::json::Json::Number(3.0)));
        assert_eq!(plates.get("counted"), Some(&crate::json::Json::Number(2.0)));
        let empty = crate::json::parse(&format!("{{{}}}", PlateSizes::default().json(1.0)))
            .expect("пустые плиты — валидный JSON");
        assert_eq!(
            empty.get("plates").unwrap().get("median_diameter_l"),
            Some(&crate::json::Json::Null)
        );
    }
}

#[cfg(test)]
mod coast_tests {
    use super::*;

    fn row(cells: &[(bool, f64)]) -> Vec<(bool, f64, f64)> {
        // (суша, осадки) при φ = 15°: (land, phi, p)
        cells.iter().map(|&(land, p)| (land, 15.0, p)).collect()
    }

    #[test]
    fn east_and_west_coasts_are_separated() {
        let mut c = CoastAsymmetry::default();
        c.push_row(row(&[(true, 0.0); 5]));
        c.push_row(row(&[
            (false, 0.0),
            (true, 100.0),
            (true, 50.0),
            (true, 200.0),
            (false, 0.0),
        ]));
        c.push_row(row(&[(true, 0.0); 5]));
        // Западная точка — 100 мм, восточная — 200 мм; средняя суша внутри.
        assert_eq!(c.east_count, 1);
        assert_eq!(c.west_count, 1);
        assert_eq!(c.east_mean(), Some(200.0));
        assert_eq!(c.west_mean(), Some(100.0));
        assert_eq!(c.ratio(), Some(2.0));
        // Точек мало для вердикта: «нет данных», а не провал.
        assert_eq!(c.in_target(), None);
    }

    #[test]
    fn north_only_coast_and_out_of_band_are_ignored() {
        let mut c = CoastAsymmetry::default();
        let mut north = row(&[(true, 0.0); 5]);
        north[2].0 = false; // вода только с севера у средней точки
        c.push_row(north);
        c.push_row(row(&[(true, 500.0); 5]));
        c.push_row(row(&[(true, 0.0); 5]));
        assert_eq!(c.east_count + c.west_count, 0);

        let mut c = CoastAsymmetry::default();
        let mut mid = row(&[
            (false, 0.0),
            (true, 100.0),
            (true, 50.0),
            (true, 200.0),
            (false, 0.0),
        ]);
        for cell in &mut mid {
            cell.1 = 30.0; // вне пояса пассатов 10–25°
        }
        c.push_row(row(&[(true, 0.0); 5]));
        c.push_row(mid);
        c.push_row(row(&[(true, 0.0); 5]));
        assert_eq!(c.east_count + c.west_count, 0);
        assert_eq!(c.ratio(), None);
    }

    #[test]
    fn verdict_needs_min_samples_on_both_sides() {
        let land = row(&[(true, 0.0); 5]);
        let coast = row(&[
            (false, 0.0),
            (true, 100.0),
            (true, 50.0),
            (true, 200.0),
            (false, 0.0),
        ]);
        let mut c = CoastAsymmetry::default();
        c.push_row(land.clone());
        for _ in 0..ZONAL_MIN_SAMPLES {
            c.push_row(coast.clone());
            c.push_row(land.clone());
        }
        assert!(c.east_count >= ZONAL_MIN_SAMPLES);
        assert!(c.west_count >= ZONAL_MIN_SAMPLES);
        assert_eq!(c.in_target(), Some(true));
    }

    #[test]
    fn coast_json_is_parseable() {
        let value = crate::json::parse(&format!("{{{}}}", CoastAsymmetry::default().json()))
            .expect("берега — валидный JSON");
        let coast = value.get("coast_asymmetry").expect("ключ coast_asymmetry");
        assert_eq!(coast.get("in_target"), Some(&crate::json::Json::Null));
        assert_eq!(
            coast.get("east_samples"),
            Some(&crate::json::Json::Number(0.0))
        );
    }
}

#[cfg(test)]
mod shadow_tests {
    use super::*;

    // Ячейка строки тени: (суша, осадки, M, ветер x, ветер z).
    fn cell(land: bool, p: f64, mountain: f64, wind: (f64, f64)) -> (bool, f64, f64, f64, f64) {
        (land, p, mountain, wind.0, wind.1)
    }

    #[test]
    fn lee_side_is_drier_downwind() {
        let plain = |p: f64| cell(true, p, 0.0, (1.0, 0.0));
        let mut s = RainShadow::default();
        s.push_row(vec![plain(900.0); 5]);
        s.push_row(vec![
            plain(900.0),
            plain(1000.0),
            cell(true, 800.0, 0.8, (1.0, 0.0)),
            plain(500.0),
            plain(900.0),
        ]);
        s.push_row(vec![plain(900.0); 5]);
        assert_eq!(s.ridge_count, 1);
        assert_eq!(s.windward_mean(), Some(1000.0));
        assert_eq!(s.leeward_mean(), Some(500.0));
        assert_eq!(s.drop_pct(), Some(50.0));
        assert_eq!(s.in_target(), None); // точек мало для вердикта
    }

    #[test]
    fn low_mountains_and_calm_are_ignored() {
        let plain = |p: f64| cell(true, p, 0.0, (1.0, 0.0));
        let mut s = RainShadow::default();
        s.push_row(vec![plain(900.0); 5]);
        s.push_row(vec![
            plain(900.0),
            plain(1000.0),
            cell(true, 800.0, 0.6, (1.0, 0.0)), // M не выше 0.6
            cell(true, 800.0, 0.9, (0.0, 0.0)), // хребет без ветра
            plain(900.0),
        ]);
        s.push_row(vec![plain(900.0); 5]);
        assert_eq!(s.ridge_count, 0);
        assert_eq!(s.drop_pct(), None);
    }

    #[test]
    fn verdict_needs_min_ridge_samples() {
        let plain = |p: f64| cell(true, p, 0.0, (1.0, 0.0));
        let ridge = vec![
            plain(900.0),
            plain(1000.0),
            cell(true, 800.0, 0.8, (1.0, 0.0)),
            plain(500.0),
            plain(900.0),
        ];
        let mut s = RainShadow::default();
        s.push_row(vec![plain(900.0); 5]);
        for _ in 0..ZONAL_MIN_SAMPLES {
            s.push_row(ridge.clone());
            s.push_row(vec![plain(900.0); 5]);
        }
        assert!(s.ridge_count >= ZONAL_MIN_SAMPLES);
        assert_eq!(s.in_target(), Some(true));
    }

    #[test]
    fn shadow_json_is_parseable() {
        let value = crate::json::parse(&format!("{{{}}}", RainShadow::default().json()))
            .expect("тень — валидный JSON");
        let shadow = value.get("rain_shadow").expect("ключ rain_shadow");
        assert_eq!(shadow.get("in_target"), Some(&crate::json::Json::Null));
        assert_eq!(
            shadow.get("ridge_samples"),
            Some(&crate::json::Json::Number(0.0))
        );
    }
}

#[cfg(test)]
mod dust_tests {
    use super::*;

    fn grid(n: usize, cells: &[(usize, usize)]) -> Vec<bool> {
        let mut land = vec![false; n * n];
        for &(i, j) in cells {
            land[j * n + i] = true;
        }
        land
    }

    #[test]
    fn small_island_counts_big_and_border_do_not() {
        // Клетка 0.001 L²: порог клочка — меньше 10 клеток.
        let mut d = IslandDust::default();
        // Островок из 2 клеток внутри и большой блок 4×4 у края.
        let mut cells = vec![(2, 2), (3, 2)];
        for j in 0..4 {
            for i in 0..4 {
                cells.push((6 + i, j));
            }
        }
        d.add_patch(&grid(12, &cells), 12, 0.001);
        assert_eq!(d.specks, 1);
        assert!((d.area_l2 - 0.144).abs() < 1e-12);
    }

    #[test]
    fn diagonal_cells_are_two_specks() {
        let mut d = IslandDust::default();
        d.add_patch(&grid(8, &[(2, 2), (3, 3)]), 8, 0.001);
        assert_eq!(d.specks, 2);
    }

    #[test]
    fn verdict_needs_ten_square_l() {
        let mut d = IslandDust::default();
        d.add_patch(&grid(8, &[(2, 2)]), 8, 0.001);
        assert_eq!(d.in_target(), None); // площади мало
        assert_eq!(d.per_1000_l2(), Some(1000.0 / 0.064));

        let mut big = IslandDust::default();
        big.add_patch(&vec![false; 200 * 200], 200, 0.001);
        assert_eq!(big.in_target(), Some(true)); // 40 L² без клочков
    }

    #[test]
    fn dust_json_is_parseable() {
        let value = crate::json::parse(&format!("{{{}}}", IslandDust::default().json()))
            .expect("пыль — валидный JSON");
        let dust = value.get("island_dust").expect("ключ island_dust");
        assert_eq!(dust.get("in_target"), Some(&crate::json::Json::Null));
        assert_eq!(dust.get("specks"), Some(&crate::json::Json::Number(0.0)));
    }
}
