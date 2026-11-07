//! Временная диагностика шва C в точке (−416, −32) сида 42.
//! Печатает все компоненты континентальности по обе стороны от разрыва.
//! Удаляется сразу после того, как причина подтверждена и исправлена.

use litho_core::crust::{crust_from_sample, kappa_global, kappa_plate};
use litho_core::plates::{
    boundary_type, boundary_weights, continent_fraction, plate_query, warped_point,
};
use litho_core::vanilla::continentalness;
use litho_core::{GenVersion, Mode, Params};

#[test]
fn diag_seam_components() {
    let params = Params::defaults(GenVersion::V0, Mode::Gameplay);
    let l = params.block.l_blocks;
    let seed = 42_u64;
    for (x, z) in [(-416.0_f64, -32.0), (-415.0, -32.0), (-414.0, -32.0)] {
        let point = litho_core::math::Vec2::new(x / l, z / l);
        let q = warped_point(point, seed, &params);
        let sample = plate_query(point, seed, &params).unwrap();
        let kg = kappa_global(q, seed, &params);
        let chi = continent_fraction(kg);
        let chi_a = continent_fraction(kappa_plate(&sample.a, q, seed, &params));
        let (chi_b, bt, weights) = match sample.b {
            Some(b) => {
                let cb = continent_fraction(kappa_plate(&b, q, seed, &params));
                let bt = boundary_type(&sample.a, &b, q, sample.n_ab, &params);
                (cb, Some(bt), Some(boundary_weights(bt, chi_a, cb)))
            }
            None => (0.0, None, None),
        };
        let crust = crust_from_sample(point, &sample, seed, &params);
        let c = continentalness(crust.e_km, crust.iota_l, &params);
        println!(
            "x={x:7.1} a={:>4} b={:?} c={:?} d_ab={:.4} w_b={:.4} w_c={:.4} | \
             kg={kg:8.4} chi={chi:.4} chi_a={chi_a:.4} chi_b={chi_b:.4} | \
             bt={:?} w={:?} | e={:.5} iota={:.5} kappa={:.5} M={:.4} C={:.5}",
            sample.a.id,
            sample.b.map(|b| b.id),
            sample.c.map(|c| c.id),
            sample.d_ab_l,
            sample.w_b,
            sample.w_c,
            bt,
            weights,
            crust.e_km,
            crust.iota_l,
            crust.kappa_l,
            crust.mountain,
            c
        );
    }
}
