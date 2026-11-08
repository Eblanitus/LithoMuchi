//! Метрики T100 / раздела 10.1 пересмотра. Отдельная диагностика поверхности,
//! не подмена существующих метрик 12.4 команды `stats` без новых флагов.

use litho_core::vanilla::peaks_valleys;
use litho_core::{Generator, Mode};

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
    pub hyps: [usize; 5],
    pub erosion: [usize; 7],
    pub valley: usize,
    pub relief_windows: usize,
    pub relief_median: Option<f64>,
    pub flat_windows: usize,
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
            hyps: [0; 5],
            erosion: [0; 7],
            valley: 0,
            relief_windows: 0,
            relief_median: None,
            flat_windows: 0,
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
                // До T114 озёр ещё нет; русла r >= 0.5 исключены из суши.
                if !c_land || river {
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
            "{:>10} {:<10} {:7.2} {:7.2} {:7.2} {:7.2} {:7.2} {:9.2} {:7.2} {:6}/{}",
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
            self.samples
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
             вода в руслах r≥0.5: {water:.2}% из {river} точек (R1: поверхность ниже y=63)\n",
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
        );
        if model.is_synthetic() {
            out.push_str(
                "WARNING: синтетический сплайн, НЕ ваниль. Для приёмки нужен --game-data.\n",
            );
        }
        if self.land == 0 || self.relief_windows == 0 {
            out.push_str("WARNING: нет суши/полных сухопутных окон; проценты и медиана не являются оценкой рельефа.\n");
        }
        out.push_str("Сеть стока, озёра и R2 ещё не реализованы (T110–T116, T143–T145).\n");
        out
    }
    pub fn json(&self, base: &RenderOptions, opts: &StatsOptions, model: &HeightModel) -> String {
        format!(
            "{{\n  \"seed\": {}, \"mode\": {}, \"version\": {}, \"path\": {},\n  \"height_source\": {}, \"height_data_hash\": \"{:016x}\", \"synthetic\": {},\n  \"approximation\": \"63.52 + 128*offset; no factor/jaggedness/3D noise\",\n  \"span_l\": {}, \"size\": {}, \"samples\": {}, \"land_samples\": {},\n  \"flooded_land_pct\": {}, \"hyps_pct\": {:?}, \"erosion_pct\": {:?},\n  \"valley_pct\": {}, \"relief_step\": {}, \"relief_windows\": {}, \"relief_median\": {}, \"flat_windows_pct\": {},\n  \"river_samples\": {}, \"river_water_pct\": {}\n}}\n",
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
        )
    }
}

pub const COMPACT_HEADER: &str = "      сид режим        flood%     E4%    E23%    E01%    E56%   relief128 valley% windows/samples";

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
        };
        let model = HeightModel::load(HeightSource::Synthetic).unwrap();
        let stats = HeightStats::collect(&base, &opts, &model).unwrap();
        assert_eq!(stats.samples, 64);
        assert!(stats.flooded().is_finite());
        let value = crate::json::parse(&stats.json(&base, &opts, &model)).unwrap();
        assert_eq!(value.get("synthetic"), Some(&crate::json::Json::Bool(true)));
    }
}
