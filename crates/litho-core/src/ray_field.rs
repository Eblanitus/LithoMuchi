//! T79 / Д-017: решётка высоты и ветра для луча влаги.
//!
//! Луч влаги делает 32 шага против ветра тремя лучами веера и на каждом шаге
//! спрашивает высоту рельефа и ветер. Точный расчёт коры в каждой точке луча —
//! почти вся цена колонки, и для редких колонок дальнего рельефа (LOD) её не
//! с кем делить. Здесь высота, ветер и широта считаются один раз в узлах
//! регулярной решётки и запоминаются. Ветер и широту луч читает билинейно,
//! высоту — сплайном Катмулла — Рома по 4 × 4 узлам: билинейная высота
//! занижает подъёмы вдоль луча (на шаге 0,25 L — на 6 %), и осадки за
//! хребтами систематически растут; сплайн этого сдвига не даёт.
//!
//! Узел зависит только от своих координат и параметров мира, поэтому ответ не
//! зависит от порядка запросов, потока и вытеснения из кэша. Узлы считаются
//! лениво и поодиночке: одинокая колонка трогает только узлы вдоль своих
//! лучей, а не целые тайлы.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Узлов на сторону тайла решётки.
pub const TILE_N: i64 = 16;
/// Ёмкость кэша тайлов: инфраструктура, на ответ не влияет (9.5).
pub const TILE_CACHE_CAPACITY: usize = 4096;

/// Такого набора бит среди конечных `f64` нет: это NaN.
const EMPTY: u64 = u64::MAX;

/// Что луч читает в точке: высота, ветер (x, z) и псевдоширота.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayFieldValue {
    pub e_km: f64,
    pub wind_x: f64,
    pub wind_z: f64,
    pub phi_deg: f64,
}

impl RayFieldValue {
    fn lerp(a: Self, b: Self, t: f64) -> Self {
        let mix = |x: f64, y: f64| x + (y - x) * t;
        Self {
            e_km: mix(a.e_km, b.e_km),
            wind_x: mix(a.wind_x, b.wind_x),
            wind_z: mix(a.wind_z, b.wind_z),
            phi_deg: mix(a.phi_deg, b.phi_deg),
        }
    }
}

/// Тайл `TILE_N × TILE_N` узлов. Ячейка пуста, пока её первое поле `EMPTY`.
#[derive(Debug)]
pub struct RayFieldTile {
    cells: Vec<[AtomicU64; 4]>,
}

impl RayFieldTile {
    pub fn empty() -> Arc<Self> {
        let cells = (0..TILE_N * TILE_N)
            .map(|_| std::array::from_fn(|_| AtomicU64::new(EMPTY)))
            .collect();
        Arc::new(Self { cells })
    }

    /// Готовый узел тайла или `None`, если его ещё никто не считал.
    /// Высота пишется последней и читается первой: готовая высота означает,
    /// что остальные поля уже записаны.
    pub fn get(&self, index: usize) -> Option<RayFieldValue> {
        let cell = &self.cells[index];
        let e = cell[0].load(Ordering::Acquire);
        if e == EMPTY {
            return None;
        }
        Some(RayFieldValue {
            e_km: f64::from_bits(e),
            wind_x: f64::from_bits(cell[1].load(Ordering::Relaxed)),
            wind_z: f64::from_bits(cell[2].load(Ordering::Relaxed)),
            phi_deg: f64::from_bits(cell[3].load(Ordering::Relaxed)),
        })
    }

    /// Узел тайла: готовый или посчитанный `compute`. Два потока могут
    /// посчитать один узел одновременно — оба запишут одни и те же биты.
    pub fn node<F>(&self, index: usize, compute: F) -> RayFieldValue
    where
        F: FnOnce() -> RayFieldValue,
    {
        if let Some(value) = self.get(index) {
            return value;
        }
        let cell = &self.cells[index];
        let value = compute();
        debug_assert_ne!(value.e_km.to_bits(), EMPTY);
        cell[1].store(value.wind_x.to_bits(), Ordering::Relaxed);
        cell[2].store(value.wind_z.to_bits(), Ordering::Relaxed);
        cell[3].store(value.phi_deg.to_bits(), Ordering::Relaxed);
        cell[0].store(value.e_km.to_bits(), Ordering::Release);
        value
    }
}

/// Ключ тайла и номер ячейки для узла `(ix, iz)`.
pub fn locate(ix: i64, iz: i64) -> ((i64, i64), usize) {
    let key = (ix.div_euclid(TILE_N), iz.div_euclid(TILE_N));
    let local = ix.rem_euclid(TILE_N) + iz.rem_euclid(TILE_N) * TILE_N;
    (key, local as usize)
}

/// Билинейное значение в точке `(gx, gz)` в шагах решётки; `node` отдаёт узел.
pub fn bilinear<F>(gx: f64, gz: f64, mut node: F) -> RayFieldValue
where
    F: FnMut(i64, i64) -> RayFieldValue,
{
    let fx0 = gx.floor();
    let fz0 = gz.floor();
    let (ix, iz) = (fx0 as i64, fz0 as i64);
    let (tx, tz) = (gx - fx0, gz - fz0);
    let near = RayFieldValue::lerp(node(ix, iz), node(ix + 1, iz), tx);
    let far = RayFieldValue::lerp(node(ix, iz + 1), node(ix + 1, iz + 1), tx);
    RayFieldValue::lerp(near, far, tz)
}

/// Сплайн Катмулла — Рома через `p1` и `p2`, `t` ∈ [0, 1].
fn catmull_rom(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
    let b = 2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3;
    let c = p2 - p0;
    0.5 * (2.0 * p1 + t * (c + t * (b + t * a)))
}

/// Высота в точке `(gx, gz)` в шагах решётки: сплайн по 4 × 4 узлам.
/// В узле совпадает с его высотой; порядок чтения узлов фиксирован.
pub fn height<F>(gx: f64, gz: f64, mut node: F) -> f64
where
    F: FnMut(i64, i64) -> RayFieldValue,
{
    let fx0 = gx.floor();
    let fz0 = gz.floor();
    let (ix, iz) = (fx0 as i64, fz0 as i64);
    let (tx, tz) = (gx - fx0, gz - fz0);
    let mut rows = [0.0_f64; 4];
    for (k, row) in rows.iter_mut().enumerate() {
        let z = iz + k as i64 - 1;
        *row = catmull_rom(
            node(ix - 1, z).e_km,
            node(ix, z).e_km,
            node(ix + 1, z).e_km,
            node(ix + 2, z).e_km,
            tx,
        );
    }
    catmull_rom(rows[0], rows[1], rows[2], rows[3], tz)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(k: f64) -> RayFieldValue {
        RayFieldValue {
            e_km: k,
            wind_x: k + 1.0,
            wind_z: k + 2.0,
            phi_deg: k + 3.0,
        }
    }

    #[test]
    fn node_is_computed_once_and_then_read_back() {
        let tile = RayFieldTile::empty();
        let mut calls = 0;
        let first = tile.node(5, || {
            calls += 1;
            value(-0.25)
        });
        let second = tile.node(5, || {
            calls += 1;
            value(9.0)
        });
        assert_eq!(first, value(-0.25));
        assert_eq!(second, first);
        assert_eq!(calls, 1);
    }

    #[test]
    fn locate_covers_negative_coordinates_without_gaps() {
        assert_eq!(locate(0, 0), ((0, 0), 0));
        assert_eq!(locate(15, 15), ((0, 0), 255));
        assert_eq!(locate(16, 0), ((1, 0), 0));
        assert_eq!(locate(-1, -1), ((-1, -1), 255));
        assert_eq!(locate(-16, -17), ((-1, -2), 15 * 16));
    }

    #[test]
    fn bilinear_is_exact_at_nodes_and_linear_between() {
        let node = |ix: i64, iz: i64| value(ix as f64 + 10.0 * iz as f64);
        assert_eq!(bilinear(3.0, -2.0, node), node(3, -2));
        let mid = bilinear(3.5, -1.5, node);
        assert!((mid.e_km - (3.5 - 15.0)).abs() < 1e-12);
        assert!((mid.phi_deg - (3.5 - 15.0 + 3.0)).abs() < 1e-12);
    }

    #[test]
    fn spline_height_is_exact_at_nodes_and_on_planes() {
        let plane = |ix: i64, iz: i64| value(0.5 * ix as f64 - 2.0 * iz as f64);
        assert_eq!(height(3.0, -2.0, plane), plane(3, -2).e_km);
        assert!((height(3.25, -1.75, plane) - (0.5 * 3.25 + 2.0 * 1.75)).abs() < 1e-12);
        // Парабола x²: сплайн держит её в середине клетки с ошибкой 1/8 от
        // второй разности, билинейная смесь — вдвое хуже.
        let bowl = |ix: i64, _: i64| value((ix * ix) as f64);
        let exact = 2.5 * 2.5;
        let spline = height(2.5, 0.0, bowl);
        let linear = bilinear(2.5, 0.0, bowl).e_km;
        assert!((spline - exact).abs() < (linear - exact).abs());
    }
}
