//! Глубинная лава у стыков плит (план 7.11, задача T53).
//! Возвращает λ ∈ [0, 1]: насколько место «горячее» для аквиферов на глубине.

use crate::math::{Vec2, dexp};
use crate::params::Params;
use crate::plates::{PlateSample, boundary_type};

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

#[cfg(test)]
mod tests {
    use super::sample;
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
}
