//! ВО0-5 (PLAN-WATER-UPDATE §6.2): разведка цены `S0` в узлах стока.
//!
//! Два числа, которых ждёт §9: сколько стоит сеть плиты с уровнями и как
//! сильно грубая решётка `S0` смещает уровень узла. Поэтому меряется:
//!
//! 1. стоимость `drainage_network` по плитам (p50/p95/max, мс) — бюджет
//!    «сеть плиты с уровнями ≤ 200 мс (p95)»;
//! 2. стоимость точного `s0_at` на узлах сети (мкс на узел) — столько стоит
//!    путь без приближения;
//! 3. ошибка быстрого пути: `S0` считается на решётке `spacing` L и
//!    интерполируется в узел билинейно. Кандидаты — путь А (0,25 L) и
//!    запасной «Б-лайт» (0,5 L). Требование §6.2: |быстрый − точный| ≤ 2
//!    блока у 95 % узлов и ≤ 6 у 99,9 %.
//!
//! Считается только `macro_y`: микробугры в уровень реки не входят (§6.3).
//! Маршрут пока старый — `S0` в узлах никто не спрашивает, разведка меряет
//! то, что появится в ВО3. Климат и морфозоны с решётки стока 1 L (тот самый
//! «быстрый `S0`» из §6.2) здесь ещё не реализованы: сначала решается,
//! проходит ли грубая решётка `S0` вообще, иначе незачем строить решётку
//! климата.

use std::collections::HashMap;
use std::time::Instant;

use litho_core::Generator;
use litho_core::plates::plates_near;

/// Допуск §6.2: 2 блока у 95 % узлов, 6 у 99,9 %.
pub const TOL_P95: f64 = 2.0;
pub const TOL_P999: f64 = 6.0;

/// Бюджет сети плиты с уровнями, мс (p95).
pub const NET_BUDGET_MS: f64 = 200.0;

/// Итог одного кандидата: решётка `spacing_l` и ошибки в блоках.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lattice {
    /// Шаг решётки в L.
    pub spacing_l: f64,
    /// Сколько выборок `s0_at` понадобилось на всё окно.
    pub samples: usize,
    /// Время на эти выборки, мс.
    pub build_ms: f64,
    /// |быстрый − точный| по узлам, блоки.
    pub errors: Vec<f64>,
}

impl Lattice {
    pub fn pct(&self, p: f64) -> f64 {
        percentile(&self.errors, p)
    }

    pub fn p95(&self) -> f64 {
        self.pct(95.0)
    }

    pub fn p999(&self) -> f64 {
        self.pct(99.9)
    }

    pub fn max(&self) -> f64 {
        self.errors.iter().copied().fold(0.0, f64::max)
    }

    /// Проходит ли кандидат допуск §6.2.
    pub fn meets(&self) -> bool {
        !self.errors.is_empty() && self.p95() <= TOL_P95 && self.p999() <= TOL_P999
    }

    /// Выборок решётки на один узел — во столько раз дешевле точного пути.
    pub fn samples_per_node(&self, nodes: usize) -> f64 {
        if nodes == 0 {
            0.0
        } else {
            self.samples as f64 / nodes as f64
        }
    }
}

/// Итог разведки.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Survey {
    pub plates: usize,
    pub nodes: usize,
    /// Сборка сети по плитам, мс.
    pub net_ms: Vec<f64>,
    /// Полное время точного `S0` на всех узлах, мс.
    pub exact_ms: f64,
    pub lattices: Vec<Lattice>,
}

impl Survey {
    pub fn net_pct(&self, p: f64) -> f64 {
        percentile(&self.net_ms, p)
    }

    pub fn net_p95(&self) -> f64 {
        self.net_pct(95.0)
    }

    pub fn net_max(&self) -> f64 {
        self.net_ms.iter().copied().fold(0.0, f64::max)
    }

    /// Мкс точного `S0` на узел.
    pub fn exact_us_per_node(&self) -> f64 {
        if self.nodes == 0 {
            0.0
        } else {
            1000.0 * self.exact_ms / self.nodes as f64
        }
    }

    /// Какой кандидат проходит допуск: первый по возрастанию шага (сначала
    /// путь А, затем «Б-лайт»).
    pub fn choice(&self) -> Option<&Lattice> {
        self.lattices.iter().find(|lattice| lattice.meets())
    }
}

/// Разведка по плитам около `center` (блочная точка): сеть, точный `S0` и
/// кандидаты-решётки `spacings_l`.
///
/// Плиты берутся первые `plates_wanted` из `plates_near`, ошибки сборки
/// (невыгодные плиты, отсутствие узлов) пропускаются, как в `water-leaks`.
pub fn survey(
    generator: &Generator,
    center: (f64, f64),
    plates_wanted: usize,
    spacings_l: &[f64],
) -> Result<Survey, String> {
    let params = generator.params();
    let l = params.block.l_blocks;
    let shift = generator.spawn_shift();
    let point = generator.macro_point(center.0, center.1);
    let radius_l = 10.0;
    let mut survey = Survey::default();
    let mut nodes: Vec<(f64, f64)> = Vec::new();
    for plate in plates_near(point, radius_l, generator.seed(), params) {
        if survey.plates >= plates_wanted {
            break;
        }
        let start = Instant::now();
        let Ok(net) = generator.drainage_network(&plate) else {
            continue;
        };
        survey.net_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        survey.plates += 1;
        for node in net.nodes() {
            if !node.in_plate {
                continue;
            }
            // `point` — макро-координата со сдвигом спавна; в блоки — обратно.
            nodes.push(((node.point.x - shift.x) * l, (node.point.z - shift.z) * l));
        }
    }
    if survey.plates == 0 {
        return Err("сеть плит не собралась: около центра нет узлов".into());
    }
    if nodes.is_empty() {
        return Err("сеть плит не дала узлов в плитах".into());
    }
    // Точный `S0`: только узлы, для которых он определён (климат и плита есть).
    let start = Instant::now();
    let mut exact: Vec<f64> = Vec::with_capacity(nodes.len());
    let mut kept: Vec<(f64, f64)> = Vec::with_capacity(nodes.len());
    for (x, z) in nodes {
        if let Some(sample) = generator.s0_at(x, z) {
            kept.push((x, z));
            exact.push(sample.macro_y);
        }
    }
    survey.exact_ms = start.elapsed().as_secs_f64() * 1000.0;
    if kept.is_empty() {
        return Err("`s0_at` не дал значений ни на одном узле".into());
    }
    survey.nodes = kept.len();
    for spacing_l in spacings_l {
        survey.lattices.push(lattice_error(
            generator, &kept, &exact, *spacing_l, l, center,
        ));
    }
    Ok(survey)
}

/// Ошибка билинейной интерполяции `S0` с решётки `spacing_l` на узлах.
///
/// Выборки берутся лениво и кэшируются: меряется ровно та цена, которую
/// заплатит генератор, спросивший решётку под узлы.
fn lattice_error(
    generator: &Generator,
    nodes: &[(f64, f64)],
    exact: &[f64],
    spacing_l: f64,
    l: f64,
    center: (f64, f64),
) -> Lattice {
    let step = spacing_l * l;
    let mut lattice = Lattice {
        spacing_l,
        ..Lattice::default()
    };
    if step <= 0.0 {
        return lattice;
    }
    let started = Instant::now();
    let mut cache: HashMap<(i64, i64), f64> = HashMap::new();
    let origin = center;
    let mut sample = |generator: &Generator, i: i64, j: i64| -> Option<f64> {
        if let Some(value) = cache.get(&(i, j)) {
            return Some(*value);
        }
        let x = origin.0 + i as f64 * step;
        let z = origin.1 + j as f64 * step;
        let value = generator.s0_at(x, z)?.macro_y;
        cache.insert((i, j), value);
        Some(value)
    };
    for ((x, z), want) in nodes.iter().zip(exact) {
        let fx = (x - origin.0) / step;
        let fz = (z - origin.1) / step;
        let i = fx.floor();
        let j = fz.floor();
        let tx = fx - i;
        let tz = fz - j;
        let (i, j) = (i as i64, j as i64);
        let corners = [
            sample(generator, i, j),
            sample(generator, i + 1, j),
            sample(generator, i, j + 1),
            sample(generator, i + 1, j + 1),
        ];
        let [Some(a), Some(b), Some(c), Some(d)] = corners else {
            // Узел без `S0` не входит в ошибку: его и точный путь не берёт.
            continue;
        };
        let fast =
            a * (1.0 - tx) * (1.0 - tz) + b * tx * (1.0 - tz) + c * (1.0 - tx) * tz + d * tx * tz;
        lattice.errors.push((fast - want).abs());
    }
    lattice.samples = cache.len();
    lattice.build_ms = started.elapsed().as_secs_f64() * 1000.0;
    lattice
}

/// Перцентиль по возрастанию; пустая выборка — 0.
pub fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = (p / 100.0).clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let low = rank.floor() as usize;
    let high = rank.ceil() as usize;
    if low == high {
        sorted[low]
    } else {
        sorted[low] + (sorted[high] - sorted[low]) * (rank - low as f64)
    }
}

/// Имя кандидата для отчёта: путь А (0,25 L) или «Б-лайт» (0,5 L).
pub fn path_name(spacing_l: f64) -> &'static str {
    if (spacing_l - 0.25).abs() < 1e-9 {
        "путь А"
    } else if (spacing_l - 0.5).abs() < 1e-9 {
        "Б-лайт"
    } else {
        "своя решётка"
    }
}

/// Строка решения: проходит ли путь А, иначе «Б-лайт», иначе «не проходит».
pub fn decision(survey: &Survey) -> String {
    match survey.choice() {
        Some(lattice) => format!(
            "{} ({} L): p95 {:.2} блока, p99.9 {:.2} — проходит допуск 6.2",
            path_name(lattice.spacing_l),
            lattice.spacing_l,
            lattice.p95(),
            lattice.p999()
        ),
        None => "ни 0,25 L, ни 0,5 L не укладываются в 2 / 6 блоков: нужен «быстрый S0» \
                 с решётки стока 1 L (климат и морозоны), а не грубая решётка S0"
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_interpolates_between_neighbors() {
        let values = [0.0, 1.0, 2.0, 3.0];
        assert_eq!(percentile(&values, 0.0), 0.0);
        assert_eq!(percentile(&values, 100.0), 3.0);
        assert!((percentile(&values, 50.0) - 1.5).abs() < 1e-12);
    }

    #[test]
    fn lattice_meets_needs_both_percentiles() {
        let mut lattice = Lattice {
            spacing_l: 0.25,
            errors: vec![1.0; 1000],
            ..Lattice::default()
        };
        assert!(lattice.meets());
        // Один выброс выше 6 блоков на 1000 значений 99,9 % не двигает:
        // перцентиль интерполирует между соседями. Ломает его группа выбросов.
        lattice.errors[999] = 10.0;
        assert!(lattice.p95() <= TOL_P95);
        assert!(lattice.meets());
        for value in lattice.errors.iter_mut().skip(990) {
            *value = 10.0;
        }
        assert!(lattice.p95() <= TOL_P95);
        assert!(lattice.p999() > TOL_P999);
        assert!(!lattice.meets());
    }

    #[test]
    fn path_names_are_stable() {
        assert_eq!(path_name(0.25), "путь А");
        assert_eq!(path_name(0.5), "Б-лайт");
        assert_eq!(path_name(0.125), "своя решётка");
    }
}
