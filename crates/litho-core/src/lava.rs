//! Глубинная лава у стыков плит (план 7.11, задача T53).
//! Возвращает λ ∈ [0, 1]: насколько место «горячее» для аквиферов на глубине.

use crate::math::{Vec2, dexp};
use crate::params::Params;
use crate::plates::{PlateSample, boundary_type};
use crate::seed::{CellRng, SALT_LAVA_AQUIFER, cell_hash, subseed};

fn boundary_term(
    sample: &PlateSample,
    neighbor: Option<crate::plates::Plate>,
    d_l: f64,
    normal: Vec2,
    weight: f64,
    params: &Params,
) -> f64 {
    let Some(b) = neighbor else {
        return 0.0;
    };
    let bt = boundary_type(&sample.a, &b, sample.q, normal, params);
    let gamma = bt.conv.max(bt.div);
    let distance_blocks = d_l * params.block.l_blocks;
    let t = distance_blocks / params.lava.width_blocks;
    weight * (params.lava.base_factor + params.lava.boundary_factor * gamma) * dexp(-t * t)
}

/// λ = max(вклад границы B, вклад границы C, фон). Вклад горячих точек в v0
/// не считается: у них нет отдельного поля в `CrustSample` (см. Д-015).
pub fn sample(point: Vec2, plates: &PlateSample, seed: u64, params: &Params) -> f64 {
    let _ = (point, seed);
    let b = boundary_term(
        plates,
        plates.b,
        plates.d_ab_l,
        plates.n_ab,
        plates.w_b,
        params,
    );
    let c = boundary_term(
        plates,
        plates.c,
        plates.d_ac_l,
        plates.n_ac,
        plates.w_c,
        params,
    );
    b.max(c).max(params.lava.background).min(1.0)
}

/// Сторона ячейки аквифера Minecraft по X/Z и по Y, блоков. Игра спрашивает
/// поле лавы в координатах ячейки: `(x div 64, y div 40, z div 64)`.
pub const AQUIFER_CELL_XZ: i64 = 64;
pub const AQUIFER_CELL_Y: i64 = 40;
/// Значение «в ячейке лава». Игра ставит лаву при `|значение| > 0.3`.
pub const AQUIFER_LAVA_ON: f64 = 0.5;
pub const AQUIFER_LAVA_OFF: f64 = 0.0;

/// T77, часть 2 (Д-098): ниже этого λ глубинной лавы нет (план 7.11).
pub const DEEP_LAVA_MIN: f64 = 0.5;
/// Ванильный уровень сплошной лавы: лавой залито всё ниже этой высоты.
pub const DEEP_LAVA_LEVEL: i64 = -54;
/// Ячейки аквифера с центром не выше этой высоты считаются глубинными
/// (слой ячеек `y div 40 ≤ −2`).
pub const DEEP_CELL_TOP: i64 = -41;
/// Поверхность не ниже этой высоты считается сушей: под морем игра сама
/// затапливает пещеры водой, и подмена затопленности там ненадёжна.
pub const DEEP_LAND_MIN_Y: f64 = 60.0;

/// Затопленность «залито до общего уровня» (порог игры не выше 0,8).
pub const FLOOD_FULL: f64 = 1.0;
/// Затопленность «свой уровень ячейки»: между порогами 0,4 и 0,8 суши.
pub const FLOOD_POCKET: f64 = 0.45;
/// Затопленность «сухо» (ниже любого порога игры).
pub const FLOOD_DRY: f64 = -1.0;
/// Разброс уровня, дающий в слое `y div 40 = −2` ровно −54:
/// `−60 + floor(0,75·10 / 3)·3 = −54`.
pub const SPREAD_DEEP: f64 = 0.75;
/// Сдвиг разброса в лавовой ячейке: ровно один шаг уровня (3 блока), чтобы
/// лава и вода соседних ячеек не стояли на одной высоте без стенки.
pub const SPREAD_LAVA_SHIFT: f64 = 0.3;
/// Исключение аквифера «не исключено».
pub const EXCLUSION_OFF: f64 = -1.0;
/// Барьер, при котором игра ставит стенку между ячейками разного уровня.
pub const BARRIER_WALL: f64 = 4.0;
/// Ширина полосы у границы ячеек лавы и воды, где ставится стенка.
pub const BARRIER_MARGIN_XZ: i64 = 16;
pub const BARRIER_MARGIN_Y: i64 = 12;

/// T77: лава аквиферов. Ячейка лавовая с вероятностью λ: у стыка плит
/// почти всегда, вдали — с фоновой долей `lava.background`. Жребий ячейки
/// зависит только от сида и её координат.
pub fn aquifer_value(lambda: f64, cell: [i64; 3], seed: u64, _params: &Params) -> f64 {
    let hash = cell_hash(subseed(seed, SALT_LAVA_AQUIFER), cell[0], cell[1], cell[2]);
    let lot = CellRng::from_hash(hash).next_unit();
    if lambda > lot {
        AQUIFER_LAVA_ON
    } else {
        AQUIFER_LAVA_OFF
    }
}

#[cfg(test)]
mod tests {
    use super::{AQUIFER_LAVA_OFF, AQUIFER_LAVA_ON, aquifer_value, sample};
    use crate::math::Vec2;
    use crate::mode::Mode;
    use crate::params::Params;
    use crate::plates::{Plate, PlateSample};
    use crate::version::GenVersion;

    fn empty_sample() -> PlateSample {
        PlateSample {
            a: Plate {
                id: 1,
                cell_i: 0,
                cell_j: 0,
                index: 0,
                seed_pos: Vec2::default(),
                center: Vec2::default(),
                has_continent: true,
                radius_l: 10.0,
                aspect: 1.0,
                axis: Vec2::new(1.0, 0.0),
                velocity: Vec2::default(),
                omega: 0.0,
                age: 0.5,
            },
            b: None,
            c: None,
            d_ab_l: 1.0,
            d_ac_l: 2.0,
            n_ab: Vec2::new(1.0, 0.0),
            n_ac: Vec2::new(0.0, 1.0),
            w_b: 0.0,
            w_c: 0.0,
            q: Vec2::default(),
        }
    }

    #[test]
    fn lava_is_at_least_background_and_bounded() {
        let p = Params::defaults(GenVersion::V0, Mode::Gameplay);
        let value = sample(Vec2::default(), &empty_sample(), 42, &p);
        assert!((value - p.lava.background).abs() < 1e-12);
        assert!((0.0..=1.0).contains(&value));
    }

    #[test]
    fn aquifer_lava_share_follows_lambda() {
        let p = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        for (lambda, lo, hi) in [
            (0.0, 0.0, 0.0),
            (0.15, 0.12, 0.18),
            (0.7, 0.67, 0.73),
            (1.0, 1.0, 1.0),
        ] {
            let mut on = 0_u32;
            let mut total = 0_u32;
            for x in -20..20 {
                for z in -20..20 {
                    for y in -2..0 {
                        let v = aquifer_value(lambda, [x, y, z], 42, &p);
                        assert!(v == AQUIFER_LAVA_ON || v == AQUIFER_LAVA_OFF);
                        on += u32::from(v == AQUIFER_LAVA_ON);
                        total += 1;
                    }
                }
            }
            let share = f64::from(on) / f64::from(total);
            assert!((lo..=hi).contains(&share), "λ={lambda}: доля {share}");
        }
        // Жребий ячейки не зависит от порядка вызовов.
        let a = aquifer_value(0.5, [3, -1, -7], 42, &p);
        let _ = aquifer_value(0.5, [4, -1, -7], 42, &p);
        assert_eq!(a, aquifer_value(0.5, [3, -1, -7], 42, &p));
    }
}
