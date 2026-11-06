//! Решётка и интерполяция (план 6.6, задача T13).
//!
//! Каркас Э1g: тайлы, сглаживание и Catmull–Rom уже есть и проверены тестами;
//! подключение к быстрому пути генератора (T55) — следующий шаг.
//!
//! Узлы привязаны к мировым координатам, тайлы 16×16 узлов с полем по 2 узла.
//! Значения сглаживаются сепарабельным ядром `[1,4,6,4,1]/16`, а в блоке
//! берутся бикубической интерполяцией Catmull–Rom по 4×4 узлам.

#![allow(clippy::needless_range_loop)]

/// Узлов в тайле по одной стороне (без поля).
pub const TILE: i64 = 16;
/// Поле с каждой стороны.
pub const HALO: i64 = 2;
/// Узлов в тайле вместе с полем.
pub const NODES: usize = 20;
/// Всего узлов в тайле.
pub const CELLS: usize = NODES * NODES;

const SMOOTH: [f64; 5] = [1.0, 4.0, 6.0, 4.0, 1.0];

pub type Nodes<const N: usize> = Vec<[f64; N]>;

/// Ключ тайла: узлы кратны шагу, поэтому порядок запросов не важен.
pub fn tile_key(x: f64, z: f64, step: f64) -> (i64, i64) {
    let span = step * TILE as f64;
    ((x / span).floor() as i64, (z / span).floor() as i64)
}

/// Блочные координаты узла `(i, j)` тайла `(tx, tz)`.
pub fn node_coords(tx: i64, tz: i64, i: usize, j: usize, step: f64) -> (f64, f64) {
    (
        (tx as f64 * TILE as f64 + i as f64 - HALO as f64) * step,
        (tz as f64 * TILE as f64 + j as f64 - HALO as f64) * step,
    )
}

/// Локальные координаты узла внутри тайла: `u = x/step − tx·16`.
pub fn local(x: f64, tx: i64, step: f64) -> f64 {
    x / step - tx as f64 * TILE as f64
}

/// Сепарабельное сглаживание с зажатием краёв: константа не меняется,
/// крупные формы тоже, а мелкие разрывы у тройных сочленений уходят.
pub fn smooth<const N: usize>(values: &mut Nodes<N>) {
    let mut pass = values.clone();
    for i in 0..NODES {
        for j in 0..NODES {
            let mut acc = [0.0_f64; N];
            for (k, weight) in SMOOTH.iter().enumerate() {
                let jj = (j as i64 + k as i64 - 2).clamp(0, NODES as i64 - 1) as usize;
                for n in 0..N {
                    acc[n] += weight * pass[i * NODES + jj][n];
                }
            }
            for n in 0..N {
                values[i * NODES + j][n] = acc[n] / 16.0;
            }
        }
    }
    pass.clone_from(values);
    for i in 0..NODES {
        for j in 0..NODES {
            let mut acc = [0.0_f64; N];
            for (k, weight) in SMOOTH.iter().enumerate() {
                let ii = (i as i64 + k as i64 - 2).clamp(0, NODES as i64 - 1) as usize;
                for n in 0..N {
                    acc[n] += weight * pass[ii * NODES + j][n];
                }
            }
            for n in 0..N {
                values[i * NODES + j][n] = acc[n] / 16.0;
            }
        }
    }
}

fn catmull(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    0.5 * (2.0 * p1
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t * t * t)
}

fn index(base: i64, offset: i64) -> usize {
    (base + offset + HALO).clamp(0, NODES as i64 - 1) as usize
}

/// Бикубическая интерполяция по 4×4 узлам; `u, v` — локальные координаты.
pub fn catmull_rom<const N: usize>(values: &Nodes<N>, u: f64, v: f64) -> [f64; N] {
    let bx = u.floor() as i64;
    let bz = v.floor() as i64;
    let fx = u - bx as f64;
    let fz = v - bz as f64;
    let xs = [index(bx, -1), index(bx, 0), index(bx, 1), index(bx, 2)];
    let zs = [index(bz, -1), index(bz, 0), index(bz, 1), index(bz, 2)];
    let mut rows = [[0.0_f64; N]; 4];
    for (r, &zz) in zs.iter().enumerate() {
        for n in 0..N {
            rows[r][n] = catmull(
                values[zz * NODES + xs[0]][n],
                values[zz * NODES + xs[1]][n],
                values[zz * NODES + xs[2]][n],
                values[zz * NODES + xs[3]][n],
                fx,
            );
        }
    }
    let mut out = [0.0_f64; N];
    for n in 0..N {
        out[n] = catmull(rows[0][n], rows[1][n], rows[2][n], rows[3][n], fz);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{CELLS, HALO, NODES, catmull_rom, local, node_coords, smooth, tile_key};

    #[test]
    fn smoothing_preserves_a_constant_field() {
        let mut values = vec![[3.5_f64, -2.0_f64]; CELLS];
        smooth(&mut values);
        for cell in values {
            assert!((cell[0] - 3.5).abs() < 1e-12);
            assert!((cell[1] + 2.0).abs() < 1e-12);
        }
    }

    #[test]
    fn catmull_rom_is_exact_on_a_linear_field() {
        let mut values = vec![[0.0_f64]; CELLS];
        for i in 0..NODES {
            for j in 0..NODES {
                // Локальная координата узла = индекс минус поле.
                let u = j as f64 - HALO as f64;
                let v = i as f64 - HALO as f64;
                values[i * NODES + j][0] = 2.0 * u + 3.0 * v;
            }
        }
        let got = catmull_rom(&values, 7.25, 11.75);
        assert!((got[0] - (2.0 * 7.25 + 3.0 * 11.75)).abs() < 1e-9);
    }

    #[test]
    fn nodes_are_tied_to_world_coordinates() {
        let step = 15.625;
        let (tx, tz) = tile_key(-1.0, -1.0, step);
        assert_eq!((tx, tz), (-1, -1));
        let (x0, z0) = node_coords(tx, tz, 2, 2, step);
        assert_eq!(x0, -16.0 * step);
        assert_eq!(z0, -16.0 * step);
        assert_eq!(local(x0, tx, step), 0.0);
    }
}
