//! Reproductions from the author's 2026-11-08 playtest, not in-game biome IDs.
use litho_core::{GenVersion, Generator, Mode};

const SEED: i64 = -7_249_957_072_115_989_875;

#[test]
fn snowy_center_on_reported_seed_is_not_directly_mapped_to_hot_desert() {
    let g = Generator::new(SEED, Mode::Gameplay, GenVersion::V0).unwrap();
    let s = g.world_sample(1287.0, 832.0).unwrap();
    assert!(s.climate.temperature.t_bio < 3.0);
    assert!(s.climate.precip.p_mm < 50.0);
    assert_eq!(s.class.cell.t, 0);
    assert_eq!(s.class.cell.biome, "snowy_plains");
    assert_eq!(s.vanilla.t, -0.70);
}

// Deliberately ignored until the boundary-field algorithm is corrected. Run
// explicitly to reproduce the known failure; do not update an expected snapshot
// to bless this discontinuity. It is already present before the column cache.
#[test]
#[ignore = "Known B/C neighbor-order discontinuity; see docs/DIAGNOSIS-2026-11-08.md"]
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
