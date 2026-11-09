//! Метрики T100 / раздела 10.1 пересмотра. Отдельная диагностика поверхности,
//! не подмена существующих метрик 12.4 команды `stats` без новых флагов.

use litho_core::vanilla::peaks_valleys;
use litho_core::{Generator, Mode, SurfaceKind};

use crate::height::{HeightInput, HeightModel, SEA_LEVEL};
use crate::render::RenderOptions;

const E_EDGES: [f64; 6] = [-0.78, -0.375, -0.2225, 0.05, 0.45, 0.55];
const HYPS_EDGES: [f64; 4] = [63.0, 70.0, 90.0, 130.0];

#[derive(Clone, Debug)]
pub struct StatsOptions {
    pub size: usize,
    pub span_l: f64,
    pub relief_size: usize,
    pub relief_step: f64,
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
    pub relief_windows: usize,
    pub relief_median: Option<f64>,
    pub flat_windows: usize,
    pub boundary: BoundaryMetrics,
}
/// T134 / 10.1: метрики границ на отдельной сетке с шагом ≤32 блока.
/// Суша = C≥порога без рек/озёр. Скачок влажности и ветер считаются на парах
/// ровно через 32 блока по обеим осям. Прямолинейность пока ищет осевые
/// участки границ длиннее 512 блоков (отклонение 0<16); диагональная лестница
/// не засчитывается как прямая, поэтому это консервативная нижняя оценка.
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
    pub wind_max_deg: f64,
    pub wind_pairs: usize,
    pub boundary_edges: usize,
    pub straight_edges: usize,
    pub thin_land: usize,
    pub min_strip_blocks: Option<f64>,
}
impl BoundaryMetrics {
    fn empty(size: usize, step: f64, l_blocks: f64) -> Self {
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
        self.humidity_jump_land = jump.iter().filter(|&&v| v).count();
        self.climate_humidity_jump_land = climate_jump.iter().filter(|&&v| v).count();
        // Осевые рёбра границ и прямые прогоны >512 блоков. Помечаем сами
        // рёбра, а не клетки, чтобы доля длины не превышала 100%.
        let mut h_edge = vec![false; n * n.saturating_sub(1)];
        let mut v_edge = vec![false; n * n.saturating_sub(1)];
        for j in 0..n {
            for i in 0..n.saturating_sub(1) {
                let a = at(i, j);
                let b = at(i + 1, j);
                h_edge[j * (n - 1) + i] = land[a] && land[b] && class[a] != class[b];
            }
        }
        for i in 0..n {
            for j in 0..n.saturating_sub(1) {
                let a = at(i, j);
                let b = at(i, j + 1);
                v_edge[j * n + i] = land[a] && land[b] && class[a] != class[b];
            }
        }
        self.boundary_edges =
            h_edge.iter().filter(|&&v| v).count() + v_edge.iter().filter(|&&v| v).count();
        let pair_h = |k: usize| -> u16 {
            let j = k / (n - 1);
            let i = k % (n - 1);
            let a = at(i, j);
            let b = at(i + 1, j);
            u16::from(class[a].min(class[b])) * 25 + u16::from(class[a].max(class[b]))
        };
        let pair_v = |k: usize| -> u16 {
            let j = k / n;
            let i = k % n;
            let a = at(i, j);
            let b = at(i, j + 1);
            u16::from(class[a].min(class[b])) * 25 + u16::from(class[a].max(class[b]))
        };
        let mark_line =
            |idx: &[usize], edges: &[bool], pair: &dyn Fn(usize) -> u16, straight: &mut [bool]| {
                let mut start = 0;
                while start < idx.len() {
                    let k0 = idx[start];
                    if !edges[k0] {
                        start += 1;
                        continue;
                    }
                    let first = pair(k0);
                    let mut end = start + 1;
                    while end < idx.len() && edges[idx[end]] && pair(idx[end]) == first {
                        end += 1;
                    }
                    if (end - start) as f64 * self.step > 512.0 {
                        for &k in &idx[start..end] {
                            straight[k] = true;
                        }
                    }
                    start = end;
                }
            };
        let mut h_straight = vec![false; h_edge.len()];
        let mut v_straight = vec![false; v_edge.len()];
        if n > 1 {
            // h_edge — вертикальные рёбра между (i,j) и (i+1,j): прямая
            // вертикальная граница идёт по столбцу i через строки j.
            for i in 0..n - 1 {
                let idx: Vec<usize> = (0..n).map(|j| j * (n - 1) + i).collect();
                mark_line(&idx, &h_edge, &pair_h, &mut h_straight);
            }
            // v_edge — горизонтальные рёбра между (i,j) и (i,j+1): прямая
            // горизонтальная граница идёт по строке j через столбцы i.
            for j in 0..n - 1 {
                let idx: Vec<usize> = (0..n).map(|i| j * n + i).collect();
                mark_line(&idx, &v_edge, &pair_v, &mut v_straight);
            }
        }
        self.straight_edges =
            h_straight.iter().filter(|&&v| v).count() + v_straight.iter().filter(|&&v| v).count();
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
        self.thin_land = thin.iter().filter(|&&v| v).count();
    }
}

fn pct(count: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        100.0 * count as f64 / total as f64
    }
}

impl HeightStats {
    pub fn collect(
        base: &RenderOptions,
        opts: &StatsOptions,
        model: &HeightModel,
    ) -> Result<Self, String> {
        let generator =
            Generator::new(base.seed, base.mode, base.version).map_err(|e| e.to_string())?;
        let step = opts.span_l * base.mode.l_blocks() / opts.size as f64;
        let mut out = Self {
            seed: base.seed,
            mode: base.mode,
            samples: opts.size * opts.size,
            land: 0,
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
            relief_windows: 0,
            relief_median: None,
            flat_windows: 0,
            boundary: BoundaryMetrics::empty(
                opts.boundary_size,
                opts.boundary_step,
                base.mode.l_blocks(),
            ),
        };
        let land_threshold = generator.params().climate.ocean_c_threshold;
        for j in 0..opts.size {
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
                let y = model.surface_y(HeightInput::from_fields(v.c, v.e, v.w))?;
                let c_land = v.c >= land_threshold;
                let river = world.river >= 0.5;
                if c_land && river {
                    out.river_samples += 1;
                    if y < SEA_LEVEL {
                        out.river_water += 1;
                    }
                }
                let h = world.hydrology;
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
                if peaks_valleys(v.w) < -0.85 {
                    out.valley += 1;
                }
            }
        }
        // Окна 128×128 при шаге 32 считаются отдельной мелкой сеткой: грубая
        // сетка 200 L не должна изображать мелкие холмы средним значением.
        let n = opts.relief_size;
        let mut heights = vec![0.0; n * n];
        let mut land = vec![false; n * n];
        for j in 0..n {
            for i in 0..n {
                let x = base.center_x + (i as f64 - n as f64 * 0.5) * opts.relief_step;
                let z = base.center_z + (j as f64 - n as f64 * 0.5) * opts.relief_step;
                let world = if opts.fast {
                    generator.world_sample_fast(x, z)
                } else {
                    generator.world_sample(x, z)
                }
                .ok_or_else(|| format!("нет образца рельефа в ({x}, {z})"))?;
                let v = world.vanilla;
                heights[j * n + i] = model.surface_y(HeightInput::from_fields(v.c, v.e, v.w))?;
                land[j * n + i] = v.c >= land_threshold && world.river < 0.5;
            }
        }
        let mut ranges = Vec::new();
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
        let mut bclass = vec![u8::MAX; bn * bn];
        let mut bland = vec![false; bn * bn];
        let mut bclimate = vec![false; bn * bn];
        let mut bjump = vec![false; bn * bn];
        let mut wind_rows: Vec<Vec<(f64, f64)>> = vec![vec![(0.0, 0.0); bn]; boff + 1];
        for j in 0..bn {
            let row = j % (boff + 1);
            for i in 0..bn {
                let x = base.center_x + (i as f64 - bn as f64 * 0.5) * bstep;
                let z = base.center_z + (j as f64 - bn as f64 * 0.5) * bstep;
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
        self.pct(self.valley, self.land)
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
             WARNING: приближение 63.52 + 128·offset; factor/jaggedness/3D-шум не учтены.\n\
             область={span}×{span} L, шаг={step:.3} блока, точек={samples}, суша без русел={land}\n\
             затопленная суша y<63 {flood:6.2}% (быстрая цель ≤10%, v1 ≤5%)\n\
             гипсометрия <63 / 63–70 / 70–90 / 90–130 / ≥130: {h0:.2} / {h1:.2} / {h2:.2} / {h3:.2} / {h4:.2}%\n\
             E4 {e4:.2}% (быстрая цель ≤60%); E2+E3 {e23:.2}%; E0+E1 {e01:.2}%; E5+E6 {e56:.2}%\n\
             долина PV<-0.85 {valley:.2}% (v1 ≤8% gameplay / ≤6% scientific)\n\
             окна {window}×{window} блоков: медиана перепада {median:.2} (быстрая цель ≥8), <4 блоков {flat:.2}%, окон={windows}\n\
             вода в руслах r≥0.5: {water:.2}% из {river} точек (R1: поверхность ниже y=63)\n\
             T115–T116 от всех точек: вода {surface_water:.2}%, болото {swamp:.2}%, сухая котловина {dry:.2}%, эстуарий {estuary:.2}%, отложено R2 {deferred:.2}%, галерейный лес {gallery:.2}%\n",
            base.seed,
            base.mode.as_str(),
            base.version.as_str(),
            if opts.fast { "fast" } else { "reference" },
            model.source.label(),
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
            surface_water = self.pct(self.water_samples, self.samples),
            swamp = self.pct(self.swamp_samples, self.samples),
            dry = self.pct(self.dry_samples, self.samples),
            estuary = self.pct(self.estuary_samples, self.samples),
            deferred = self.pct(self.deferred_samples, self.samples),
            gallery = self.pct(self.gallery_samples, self.samples),
        );
        let b = &self.boundary;
        out.push_str(&format!(
            "T134 границы: сетка {bn}×{bn} ({bsamples} точек), шаг {bs:.3} блока, область {span:.3}×{span:.3} L, суша {bl}\n\
             скачок h≥2 всей суши {jump:.2}% точек ({jp}/{pairs} пар, {pair_pct:.2}% пар), ветер max {wind:.3}°/32 блока ({wp} пар)\n\
             климатическая суша {cl}: скачок h≥2 {cjump:.2}% точек ({cjp}/{cpairs} пар, {cpair_pct:.2}% пар); гидро-override исключён\n\
             прямолинейность осевых границ >512 блоков {straight:.2}% ({se}/{be} рёбер), полоски <32 блоков {thin:.2}% суши, min {minstrip}\n\
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
        out.push_str("T115–T116: сеть/низкие озёра подключены к C/E/W и биомам (R1). Галерейные леса T116 включены; высокие озёра и собственный PV — T141/T143/T145. Проценты воды здесь не заменяют проверку реальной высоты.\n");
        out
    }
    pub fn json(&self, base: &RenderOptions, opts: &StatsOptions, model: &HeightModel) -> String {
        format!(
            "{{\n  \"seed\": {}, \"mode\": {}, \"version\": {}, \"path\": {},\n  \"height_source\": {}, \"height_data_hash\": \"{:016x}\", \"synthetic\": {},\n  \"approximation\": \"63.52 + 128*offset; no factor/jaggedness/3D noise\",\n  \"span_l\": {}, \"size\": {}, \"samples\": {}, \"land_samples\": {},\n  \"flooded_land_pct\": {}, \"hyps_pct\": {:?}, \"erosion_pct\": {:?},\n  \"valley_pct\": {}, \"relief_step\": {}, \"relief_windows\": {}, \"relief_median\": {}, \"flat_windows_pct\": {},\n  \"river_samples\": {}, \"river_water_pct\": {}, \"gallery_samples\": {}, \"gallery_pct\": {},\n  \"boundary\": {{\"size\": {}, \"step\": {}, \"span_l\": {}, \"land\": {}, \"humidity_jump_pct\": {}, \"humidity_jump_pair_pct\": {}, \"humidity_jump_pairs\": {}, \"humidity_pairs\": {}, \"climate_land\": {}, \"climate_humidity_jump_pct\": {}, \"climate_humidity_pair_pct\": {}, \"climate_humidity_jump_pairs\": {}, \"climate_humidity_pairs\": {}, \"wind_max_deg\": {}, \"wind_pairs\": {}, \"straight_pct\": {}, \"straight_edges\": {}, \"boundary_edges\": {}, \"thin_pct\": {}, \"min_strip_blocks\": {}, \"acceptance_volume\": {}}}\n}}\n",
            self.seed,
            crate::json::quote(self.mode.as_str()),
            crate::json::quote(base.version.as_str()),
            crate::json::quote(if opts.fast { "fast" } else { "reference" }),
            crate::json::quote(&model.source.label()),
            model.data_hash,
            model.is_synthetic(),
            opts.span_l,
            opts.size,
            self.samples,
            self.land,
            self.flooded(),
            self.hyps.map(|n| self.pct(n, self.land)),
            self.erosion.map(|n| self.pct(n, self.land)),
            self.valley_pct(),
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
        };
        let opts = StatsOptions {
            size: 8,
            span_l: 8.0,
            relief_size: 8,
            relief_step: 32.0,
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
}
