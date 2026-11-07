//! Reproductions from the author's 2026-11-08 playtest, not in-game biome IDs.
use litho_core::{GenVersion, Generator, Mode};

const SEED: i64 = -7_249_957_072_115_989_875;

#[test]
fn reported_center_is_cool_without_hot_desert_fallback() {
    let g = Generator::new(SEED, Mode::Gameplay, GenVersion::V0).unwrap();
    let s = g.world_sample(1287.0, 832.0).unwrap();
    assert!(s.climate.temperature.t_bio < 6.0);
    assert!(s.climate.precip.p_mm < 100.0);
    assert!(s.class.cell.t <= 1);
    assert_ne!(s.class.cell.biome, "desert");
    assert!(s.vanilla.t < 0.0);
}

// Reproduction previously failed with a 2.37 km jump. Keep enabled after the
// symmetric boundary-pair fix; never bless the old discontinuity as a snapshot.
#[test]
fn wall_bc_swap_must_not_jump_at_infinitesimal_distance() {
    let g = Generator::new(SEED, Mode::Gameplay, GenVersion::V0).unwrap();
    let left = g.world_sample(-1681.843796630837, -610.0).unwrap();
    let right = g.world_sample(-1681.843794630836, -610.0).unwrap();
    assert!(
        (left.climate.surface.e_km - right.climate.surface.e_km).abs() < 0.001,
        "height discontinuity: {} -> {} km",
        left.climate.surface.e_km,
        right.climate.surface.e_km
    );
    assert!((left.vanilla.e - right.vanilla.e).abs() < 0.02);
}
